//! Containment limits. A count over its limit is refused with the observed
//! and permitted numbers. Limits decide no meaning.

use crate::codes::Refusal;

/// A counted quantity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Limit {
    /// Frame interpretations at one position, after longest-match.
    Interpretations,
    /// Candidates at one position: contract and frame pairs, before merging.
    Candidates,
    /// Distinct contracts actualized together at one position.
    Actualized,
}

impl Limit {
    const fn name(self) -> &'static str {
        match self {
            Self::Interpretations => "interpretations",
            Self::Candidates => "candidates",
            Self::Actualized => "actualized",
        }
    }
}

/// `Ok` when `observed` is at most `permitted`.
pub(crate) fn within(
    limit: Limit,
    observed: usize,
    permitted: u8,
    position: u32,
) -> Result<(), Refusal> {
    let permitted = usize::from(permitted);
    if observed <= permitted {
        Ok(())
    } else {
        Err(Refusal::LimitExceeded {
            position,
            limit: limit.name(),
            observed,
            permitted,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_nine_permitted_eight() {
        let refusal = within(Limit::Interpretations, 9, 8, 3).unwrap_err();
        assert_eq!(
            refusal,
            Refusal::LimitExceeded {
                position: 3,
                limit: "interpretations",
                observed: 9,
                permitted: 8
            }
        );
        assert_eq!(
            refusal.to_string(),
            "[BBBC0104] interpretations limit exceeded at position 3: observed 9, permitted 8"
        );
        assert!(within(Limit::Interpretations, 8, 8, 3).is_ok());
        assert!(within(Limit::Candidates, 0, 0, 3).is_ok());
    }
}
