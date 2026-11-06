//! The ob-serve-a-bowl adapter (feature `bowl`).
//!
//! Each step event becomes one `WakeKey64` packet published to an
//! `ObservableWakeRuntime`:
//! - `phase` holds the event kind ([`EventKind`]);
//! - `field_mask` holds one bit per changed dimension (dimensions 0 to 15;
//!   higher ones do not fit the 16-bit mask and are left out);
//! - `reason` is `manual`, an existing label;
//! - the payload holds the position (`slot`), the focal index (`generation`),
//!   the lowest authorized contract id (`offset`, `u32::MAX` when none) and,
//!   on `Authorized`, how many contracts actualize together (`len`).
//!
//! `publish` returns a `WakeReport`. The adapter drops it: nothing flows back
//! to the stepper.

use ob_serve_a_bowl::{
    ObservablePacket, ObservableWakeRuntime, PayloadRef, WakeKey64, WakeProfileCode, WakeReasonCode,
};

use crate::model::{ContractId, DimensionId};
use crate::observe::{Observer, StepEvent};

/// The event kind carried in `WakeKey64.phase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum EventKind {
    /// `StepEvent::Observed`.
    Observed = 1,
    /// `StepEvent::Framed`.
    Framed = 2,
    /// `StepEvent::Candidate`.
    Candidate = 3,
    /// `StepEvent::Refused`.
    Refused = 4,
    /// `StepEvent::Authorized`.
    Authorized = 5,
    /// `StepEvent::Committed`.
    Committed = 6,
    /// `StepEvent::Emitted`.
    Emitted = 7,
}

impl EventKind {
    /// The kind of `event`.
    #[must_use]
    pub const fn of(event: &StepEvent<'_>) -> Self {
        match event {
            StepEvent::Observed(_) => Self::Observed,
            StepEvent::Framed(_) => Self::Framed,
            StepEvent::Candidate(_) => Self::Candidate,
            StepEvent::Refused(_) => Self::Refused,
            StepEvent::Authorized(_) => Self::Authorized,
            StepEvent::Committed { .. } => Self::Committed,
            StepEvent::Emitted(_) => Self::Emitted,
        }
    }
}

/// Publishes every step event to an ob-serve-a-bowl runtime.
#[derive(Default)]
pub struct BowlObserver {
    runtime: ObservableWakeRuntime,
    position: u32,
    focal: u32,
    contract: Option<ContractId>,
    published: usize,
}

impl std::fmt::Debug for BowlObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BowlObserver")
            .field("position", &self.position)
            .field("focal", &self.focal)
            .field("published", &self.published)
            .finish_non_exhaustive()
    }
}

impl BowlObserver {
    /// An adapter over `runtime`. Register ob-serve-a-bowl observers on it first.
    #[must_use]
    pub fn new(runtime: ObservableWakeRuntime) -> Self {
        Self {
            runtime,
            ..Self::default()
        }
    }

    /// The runtime, to read what was delivered.
    #[must_use]
    pub const fn runtime(&self) -> &ObservableWakeRuntime {
        &self.runtime
    }

    /// The runtime, to drain delivered events.
    pub fn runtime_mut(&mut self) -> &mut ObservableWakeRuntime {
        &mut self.runtime
    }

    /// Packets published so far.
    #[must_use]
    pub const fn published(&self) -> usize {
        self.published
    }

    /// The packet for `event`, given the adapter's position, focal index and contract.
    fn packet(&self, event: &StepEvent<'_>) -> ObservablePacket {
        let mut mask: u16 = 0;
        let mut mark = |d: DimensionId| {
            if d.0 < 16 {
                mask |= 1 << d.0;
            }
        };
        match event {
            StepEvent::Committed { before, after } => {
                for i in 0..u16::MAX {
                    let d = DimensionId(i);
                    if !before.has(d) {
                        break;
                    }
                    if before.stack(d) != after.stack(d) || before.word(d) != after.word(d) {
                        mark(d);
                    }
                }
            }
            StepEvent::Emitted(e) => mark(e.dimension),
            _ => {}
        }
        let key = WakeKey64::new(
            WakeReasonCode::Manual.as_u64(),
            WakeProfileCode::InlineQueue.as_u64(),
            3,
            u64::from(mask),
            EventKind::of(event) as u64,
            0,
        );
        let contract = match event {
            StepEvent::Candidate(c) => Some(c.contract),
            StepEvent::Authorized(ids) => ids.first().copied(),
            StepEvent::Committed { .. } | StepEvent::Emitted(_) => self.contract,
            _ => None,
        };
        let together = match event {
            StepEvent::Authorized(ids) => ids.len(),
            _ => 0,
        };
        let payload = PayloadRef::new(
            self.position,
            self.focal,
            contract.map_or(u32::MAX, |c| u32::from(c.0)),
            u32::try_from(together).unwrap_or(u32::MAX),
        );
        ObservablePacket::new(key, payload)
    }
}

impl Observer for BowlObserver {
    fn observe(&mut self, event: &StepEvent<'_>) {
        match event {
            StepEvent::Observed(o) => {
                if self.published > 0 {
                    self.focal += 1;
                }
                self.position = o.position();
                self.contract = None;
            }
            StepEvent::Authorized(ids) => self.contract = ids.first().copied(),
            _ => {}
        }
        let packet = self.packet(event);
        let _counts = self.runtime.publish(packet);
        self.published += 1;
    }
}
