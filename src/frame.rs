//! The frame recognizer.
//!
//! It looks at one focal item and up to R items on each side, R being the
//! longest declared sequence minus one. Every declared sequence that matches
//! a window holding the focal item gives the focal item one frame word: the
//! sequence key, the offset, the [`Position`], the boundary flag, and the
//! facts declared for that offset. Raw n-grams never become bits.
//!
//! A match whose window lies inside a longer match's window is dropped
//! (longest-match). The boundary flag is set on every frame word of an item
//! that ends one match and begins another. Items past the input are the end
//! item; there is nothing before the first item.
//!
//! The recognizer sees no state and writes nothing but its own output.

use big_bad_bool_core::BitFieldSpec;
use big_bad_bool_interact::FieldValue;
use flaga_solver::Schema;

use crate::codes::Finding;
use crate::model::{
    DimensionSpec, Position, ProgramSpec, ResolvedSequence, SequenceId, field_value,
};

/// One frame interpretation of the focal item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameFacts {
    /// The matched sequence.
    pub sequence: SequenceId,
    /// The focal item's offset in it.
    pub offset: u8,
    /// The frame word. Its raw value is the frame key.
    pub word: u128,
}

/// The compiled recognizer of one program.
#[derive(Debug, Clone)]
pub struct Recognizer {
    sequences: Vec<ResolvedSequence>,
    words: Vec<Vec<u128>>,
    item_schema: Schema<'static>,
    frame_dim: DimensionSpec,
    boundary: BitFieldSpec,
    boundary_bit: u128,
    end_word: u128,
}

struct Match {
    sequence: usize,
    offset: usize,
    start: usize,
    len: usize,
}

impl Match {
    const fn end(&self) -> usize {
        self.start + self.len
    }

    const fn inside(&self, other: &Self) -> bool {
        other.len > self.len && other.start <= self.start && other.end() >= self.end()
    }
}

impl Recognizer {
    /// Compiles one frame word per sequence offset and checks each with
    /// flaga_solver, with and without the boundary flag.
    ///
    /// # Errors
    ///
    /// `IllegalWord` for every frame word its schema refuses.
    pub fn new(spec: &ProgramSpec) -> Result<Self, Vec<Finding>> {
        let frame_dim = spec.dims[spec.frame_dim];
        let layout = spec.frame;
        let mut found = Vec::new();
        let mut words = Vec::with_capacity(spec.sequences.len());
        for (i, s) in spec.sequences.iter().enumerate() {
            let key = SequenceId(i as u16).key();
            let mut per_offset = Vec::with_capacity(s.items.len());
            for (offset, facts) in s.facts.iter().enumerate() {
                let mut values = vec![
                    FieldValue::new(layout.sequence.name, key),
                    FieldValue::new(layout.offset.name, offset as u128),
                    FieldValue::new(
                        layout.position.name,
                        Position::of(offset, s.items.len()).value(),
                    ),
                ];
                values.extend_from_slice(facts);
                let word = match frame_dim.patch(frame_dim.default_word(), &values) {
                    Ok(word) => word,
                    Err(e) => {
                        found.push(Finding::IllegalWord {
                            owner: s.name,
                            dimension: frame_dim.name(),
                            verdict: e.to_string(),
                        });
                        0
                    }
                };
                per_offset.push(word);
            }
            words.push(per_offset);
        }
        let recognizer = Self {
            sequences: spec.sequences.clone(),
            words,
            item_schema: *spec.dims[spec.item_dim].schema(),
            frame_dim,
            boundary: layout.boundary,
            boundary_bit: layout.boundary_bit,
            end_word: spec.end_word,
        };
        for (s, per_offset) in recognizer.words.iter().enumerate() {
            for word in per_offset {
                for w in [*word, recognizer.with_boundary(*word)] {
                    let verdict = flaga_solver::check(frame_dim.schema(), w);
                    if !verdict.is_ok_ignoring_deprecated() {
                        found.push(Finding::IllegalWord {
                            owner: recognizer.sequences[s].name,
                            dimension: frame_dim.name(),
                            verdict: verdict.to_string(),
                        });
                    }
                }
            }
        }
        if found.is_empty() {
            Ok(recognizer)
        } else {
            Err(found)
        }
    }

    fn with_boundary(&self, word: u128) -> u128 {
        let value = field_value(&self.boundary, word) | self.boundary_bit;
        self.frame_dim
            .patch(word, &[FieldValue::new(self.boundary.name, value)])
            .unwrap_or(word)
    }

    /// The frame words of `items[focal]`. Items past the end are the end item.
    #[must_use]
    pub fn recognize(&self, items: &[u128], focal: usize) -> Vec<FrameFacts> {
        let end = self.end_word;
        self.recognize_with(focal, |i| Some(items.get(i).copied().unwrap_or(end)))
    }

    /// The frame words of the item at `focal`, reading items through `item_at`.
    /// `None` from `item_at` means no item there.
    pub(crate) fn recognize_with(
        &self,
        focal: usize,
        item_at: impl Fn(usize) -> Option<u128>,
    ) -> Vec<FrameFacts> {
        let Some(focal_item) = item_at(focal) else {
            return Vec::new();
        };
        let mut matches = Vec::new();
        for (sequence, s) in self.sequences.iter().enumerate() {
            let len = s.items.len();
            for offset in 0..len.min(focal + 1) {
                if !s.items[offset].matches(&self.item_schema, focal_item) {
                    continue;
                }
                let start = focal - offset;
                let whole = (0..len).all(|k| {
                    k == offset
                        || item_at(start + k)
                            .is_some_and(|w| s.items[k].matches(&self.item_schema, w))
                });
                if whole {
                    matches.push(Match {
                        sequence,
                        offset,
                        start,
                        len,
                    });
                }
            }
        }
        let kept: Vec<&Match> = matches
            .iter()
            .filter(|m| !matches.iter().any(|other| m.inside(other)))
            .collect();
        let boundary = kept.iter().any(|a| {
            a.offset + 1 == a.len && kept.iter().any(|b| b.offset == 0 && !std::ptr::eq(*a, *b))
        });
        kept.iter()
            .map(|m| {
                let word = self.words[m.sequence][m.offset];
                FrameFacts {
                    sequence: SequenceId(m.sequence as u16),
                    offset: m.offset as u8,
                    word: if boundary {
                        self.with_boundary(word)
                    } else {
                        word
                    },
                }
            })
            .collect()
    }

    /// Every frame word the declared sequences can produce, with and without
    /// the boundary flag.
    pub(crate) fn universe(&self) -> Vec<FrameFacts> {
        let mut all = Vec::new();
        for (s, per_offset) in self.words.iter().enumerate() {
            for (offset, word) in per_offset.iter().enumerate() {
                for w in [*word, self.with_boundary(*word)] {
                    all.push(FrameFacts {
                        sequence: SequenceId(s as u16),
                        offset: offset as u8,
                        word: w,
                    });
                }
            }
        }
        all
    }

    /// Whether `item` can be the focal item of `frame`'s sequence offset.
    pub(crate) fn accepts(&self, frame: &FrameFacts, item: u128) -> bool {
        self.sequences[usize::from(frame.sequence.0)].items[usize::from(frame.offset)]
            .matches(&self.item_schema, item)
    }
}
