// Copyright (c) 2023-present, Raphael Amorim.
//
// This source code is licensed under the MIT license found in the
// LICENSE file in the root directory of this source tree.
//
// layout_data.rs was originally retired from dfrg/swash_demo licensed under MIT
// https://github.com/dfrg/swash_demo/blob/master/LICENSE
//
// This file had updates to support color, underline_color, background_color
// and other functionalities

use crate::font::FontLibrary;
use crate::font_introspector::shape::cluster::GlyphCluster;
use crate::font_introspector::shape::cluster::OwnedGlyphCluster;
use crate::font_introspector::shape::ShapeContext;
use crate::font_introspector::text::cluster::Status;
use crate::font_introspector::text::cluster::CharCluster;
use crate::font_introspector::text::Script;
use crate::font_introspector::Metrics;
use crate::font_introspector::Synthesis;
use crate::layout::render_data::RenderData;
use lru::LruCache;
use rustc_hash::FxHashMap;
use std::num::NonZeroUsize;

use crate::font_introspector::Attributes;
use crate::font_introspector::Setting;
use crate::{sugarloaf::primitives::SugarCursor, Graphic};

/// Data that describes a fragment.
#[derive(Copy, Debug, Clone)]
pub struct FragmentData {
    /// Offset of the text.
    pub start: usize,
    /// End of the text.
    pub end: usize,
    /// Font variations.
    pub vars: FontSettingKey,
}

/// Builder Line State
#[derive(Default)]
pub struct BuilderLineText {
    /// Combined text.
    pub content: Vec<char>,
    /// Fragment index per character.
    pub frags: Vec<u32>,
    /// Span index per character.
    pub spans: Vec<usize>,
}

#[derive(Default)]
pub struct BuilderLine {
    pub text: BuilderLineText,
    /// Collection of fragments.
    pub fragments: Vec<FragmentData>,
    /// Span index per character.
    pub styles: Vec<FragmentStyle>,
}

/// Builder state.
#[derive(Default)]
pub struct BuilderState {
    /// Lines State
    pub lines: Vec<BuilderLine>,
    /// Font variation setting cache.
    pub vars: FontSettingCache<f32>,
    /// User specified scale.
    pub scale: f32,
    // Font size in ppem.
    pub font_size: f32,
}

impl BuilderState {
    /// Creates a new layout state.
    pub fn new() -> Self {
        let mut lines = vec![BuilderLine::default()];
        lines[0].styles.push(FragmentStyle::default());
        Self {
            lines,
            ..BuilderState::default()
        }
    }
    #[inline]
    pub fn new_line(&mut self) {
        self.lines.push(BuilderLine::default());
        let last = self.lines.len() - 1;
        self.lines[last].styles.push(FragmentStyle::default());
    }
    #[inline]
    pub fn current_line(&self) -> usize {
        let size = self.lines.len();
        if size == 0 {
            0
        } else {
            size - 1
        }
    }
    #[inline]
    pub fn clear(&mut self) {
        self.lines.clear();
        self.vars.clear();
    }

    #[inline]
    pub fn begin(&mut self) {
        self.lines.push(BuilderLine::default());
    }
}

/// Index into a font setting cache.
pub type FontSettingKey = u32;

/// Cache of tag/value pairs for font settings.
#[derive(Default)]
pub struct FontSettingCache<T: Copy + PartialOrd + PartialEq> {
    settings: Vec<Setting<T>>,
    lists: Vec<FontSettingList>,
    tmp: Vec<Setting<T>>,
}

impl<T: Copy + PartialOrd + PartialEq> FontSettingCache<T> {
    pub fn get(&self, key: u32) -> &[Setting<T>] {
        if key == !0 {
            &[]
        } else {
            self.lists
                .get(key as usize)
                .map(|list| list.get(&self.settings))
                .unwrap_or(&[])
        }
    }

    pub fn clear(&mut self) {
        self.settings.clear();
        self.lists.clear();
        self.tmp.clear();
    }
}

/// Sentinel for an empty set of font settings.
pub const EMPTY_FONT_SETTINGS: FontSettingKey = !0;

/// Range within a font setting cache.
#[derive(Copy, Clone)]
struct FontSettingList {
    pub start: u32,
    pub end: u32,
}

impl FontSettingList {
    pub fn get<T>(self, elements: &[T]) -> &[T] {
        elements
            .get(self.start as usize..self.end as usize)
            .unwrap_or(&[])
    }
}

#[repr(u8)]
#[derive(Copy, Clone, PartialEq, Debug, Default)]
pub enum UnderlineShape {
    #[default]
    Regular = 0,
    Dotted = 1,
    Dashed = 2,
    Curly = 3,
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub struct UnderlineInfo {
    pub offset: f32,
    pub size: f32,
    pub is_doubled: bool,
    pub shape: UnderlineShape,
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum FragmentStyleDecoration {
    // offset, size
    Underline(UnderlineInfo),
    Strikethrough,
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub struct FragmentStyle {
    pub font_id: usize,
    //  Unicode width
    pub width: f32,
    /// Font attributes.
    pub font_attrs: Attributes,
    /// Font color.
    pub color: [f32; 4],
    /// Background color.
    pub background_color: Option<[f32; 4]>,
    /// Font variations.
    pub font_vars: FontSettingKey,
    /// Additional spacing between letters (clusters) of text.
    // pub letter_spacing: f32,
    /// Additional spacing between words of text.
    // pub word_spacing: f32,
    /// Multiplicative line spacing factor.
    // pub line_spacing: f32,
    /// Enable underline decoration.
    pub decoration: Option<FragmentStyleDecoration>,
    /// Decoration color.
    pub decoration_color: Option<[f32; 4]>,
    /// Cursor style.
    pub cursor: Option<SugarCursor>,
    /// Media
    pub media: Option<Graphic>,
}

impl Default for FragmentStyle {
    fn default() -> Self {
        Self {
            font_id: 0,
            width: 1.0,
            font_attrs: Attributes::default(),
            font_vars: EMPTY_FONT_SETTINGS,
            // letter_spacing: 0.,
            // word_spacing: 0.,
            // line_spacing: 1.,
            color: [1.0, 1.0, 1.0, 1.0],
            background_color: None,
            cursor: None,
            decoration: None,
            decoration_color: None,
            media: None,
        }
    }
}

/// Context for paragraph layout.
pub struct LayoutContext {
    fonts: FontLibrary,
    font_features: Vec<crate::font_introspector::Setting<u16>>,
    scx: ShapeContext,
    state: BuilderState,
    word_cache: WordCache,
    metrics_cache: MetricsCache,
}

impl LayoutContext {
    /// Creates a new layout context with the specified font library.
    pub fn new(font_library: &FontLibrary) -> Self {
        Self {
            fonts: font_library.clone(),
            scx: ShapeContext::new(),
            state: BuilderState::new(),
            word_cache: WordCache::new(),
            font_features: vec![],
            metrics_cache: MetricsCache::default(),
        }
    }

    #[inline]
    pub fn font_library(&self) -> &FontLibrary {
        &self.fonts
    }

    #[inline]
    pub fn set_font_features(
        &mut self,
        font_features: Vec<crate::font_introspector::Setting<u16>>,
    ) {
        self.font_features = font_features;
    }

    /// Creates a new builder for computing a paragraph layout with the
    /// specified direction, language and scaling factor.
    #[inline]
    pub fn builder(&mut self, scale: f32, font_size: f32) -> ParagraphBuilder {
        self.state.clear();
        self.state.begin();
        let prev_font_size = self.state.font_size;
        self.state.scale = scale;
        self.state.font_size = font_size * scale;

        if prev_font_size != self.state.font_size {
            // self.cache.inner.clear();
            self.metrics_cache.inner.clear();
        }
        ParagraphBuilder {
            // bidi: &mut self.bidi,
            // needs_bidi: false,
            font_features: &self.font_features,
            fonts: &self.fonts,
            scx: &mut self.scx,
            s: &mut self.state,
            last_offset: 0,
            word_cache: &mut self.word_cache,
            metrics_cache: &mut self.metrics_cache,
        }
    }
}

/// Builder for computing the layout of a paragraph.
pub struct ParagraphBuilder<'a> {
    fonts: &'a FontLibrary,
    font_features: &'a Vec<crate::font_introspector::Setting<u16>>,
    scx: &'a mut ShapeContext,
    s: &'a mut BuilderState,
    last_offset: u32,
    word_cache: &'a mut WordCache,
    metrics_cache: &'a mut MetricsCache,
}

impl<'a> ParagraphBuilder<'a> {
    #[inline]
    pub fn new_line(&mut self) {
        self.s.new_line();
    }

    /// Adds a text fragment to the paragraph.
    pub fn add_text(&mut self, text: &str, style: FragmentStyle) -> Option<()> {
        let current_line = self.s.current_line();
        let line = &mut self.s.lines[current_line];
        let id = line.text.frags.len();
        let mut offset = self.last_offset;
        line.styles.push(style);
        let span_id = line.styles.len() - 1;

        let start = line.text.content.len();

        for ch in text.chars() {
            line.text.content.push(ch);
            offset += (ch).len_utf8() as u32;
        }

        // println!(">>> {:?}", text);
        let end = line.text.content.len();
        let len = end - start;
        line.text.frags.reserve(len);
        for _ in 0..len {
            line.text.frags.push(id as u32);
        }

        line.text.spans.reserve(len);
        for _ in 0..len {
            line.text.spans.push(span_id);
        }

        line.fragments.push(FragmentData {
            start,
            end,
            vars: style.font_vars,
        });

        self.last_offset = offset;
        Some(())
    }

    /// Consumes the builder and fills the specified paragraph with the result.
    pub fn build_into(mut self, render_data: &mut RenderData) {
        self.resolve(render_data);
    }

    /// Consumes the builder and returns the resulting paragraph.
    pub fn build(self) -> RenderData {
        let mut render_data = RenderData::default();
        self.build_into(&mut render_data);
        render_data
    }
}

impl<'a> ParagraphBuilder<'a> {
    fn resolve(&mut self, render_data: &mut RenderData) {
        let font_library = { &self.fonts.inner.read().unwrap() };
        for line_number in 0..self.s.lines.len() {
            let mut char_cluster = CharCluster::new();
            let line = &self.s.lines[line_number];
            for item in &line.fragments {
                let range = item.start..item.end;
                let span_index = self.s.lines[line_number].text.spans[item.start];
                let style = self.s.lines[line_number].styles[span_index];
                let vars = self.s.vars.get(item.vars);
                let mut shape_state = ShapeState {
                    script: Script::Latin,
                    features: self.font_features,
                    vars,
                    synth: font_library[style.font_id].synth,
                    // font_id: None,
                    size: self.s.font_size,
                };

                let shaper_key: String = self.s.lines[line_number].text.content
                    [range.to_owned()]
                .iter()
                .collect();
                if let Some(shaper) = self.word_cache.inner.get(&shaper_key) {
                    if let Some(metrics) =
                        self.metrics_cache.inner.get(&style.font_id)
                    {
                        if render_data.push_run_without_shaper(
                            &style,
                            shape_state.size,
                            line_number as u32,
                            shaper,
                            metrics,
                        ) {
                            continue;
                        }
                    }
                }

                self.word_cache.key = shaper_key;

                let charmap = &font_library[style.font_id].as_ref().charmap();
                let status = char_cluster.map(|ch| charmap.map(ch));
                if status != Status::Discard {
                    shape_state.synth = font_library[style.font_id].synth;
                }

                let mut shaper = self
                    .scx
                    .builder(font_library[style.font_id].as_ref())
                    .script(shape_state.script)
                    .size(shape_state.size)
                    .features(shape_state.features.iter().copied())
                    .variations(shape_state.synth.variations().iter().copied())
                    .variations(shape_state.vars.iter().copied())
                    .build();

                if !self.metrics_cache.inner.contains_key(&style.font_id) {
                    self.metrics_cache
                        .inner
                        .insert(style.font_id, shaper.metrics());
                }

                shaper.add_str(&self.word_cache.key);
                render_data.push_run(
                    &style,
                    self.s.font_size,
                    line_number as u32,
                    shaper,
                    self.word_cache,
                );
            }
        }
    }
}

struct ShapeState<'a> {
    features: &'a [Setting<u16>],
    synth: Synthesis,
    vars: &'a [Setting<f32>],
    script: Script,
    size: f32,
}

pub struct WordCache {
    pub inner: LruCache<String, Vec<OwnedGlyphCluster>>,
    stash: Vec<OwnedGlyphCluster>,
    pub key: String,
}

impl WordCache {
    pub fn new() -> Self {
        WordCache {
            inner: LruCache::new(NonZeroUsize::new(1024).unwrap()),
            stash: vec![],
            key: String::new(),
        }
    }

    #[inline]
    pub fn add_glyph_cluster(&mut self, glyph_cluster: &GlyphCluster) {
        self.stash.push(glyph_cluster.into());
    }

    #[inline]
    pub fn finish(&mut self) {
        // println!("{:?} {:?}", self.key, self.inner.len());
        if !self.key.is_empty()
            && !self.stash.is_empty()
            && self.inner.get(&self.key).is_none()
        {
            self.inner.put(
                std::mem::take(&mut self.key),
                std::mem::take(&mut self.stash),
            );
            return;
        }
        self.stash.clear();
        self.key.clear();
    }
}

#[derive(Default)]
struct MetricsCache {
    pub inner: FxHashMap<usize, Metrics>,
}
