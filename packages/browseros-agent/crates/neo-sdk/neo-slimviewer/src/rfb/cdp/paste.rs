//! Paste decisions depend on delivery evidence, never text equality.

use super::agent::{PasteError, PasteOutcome, Press};
use super::protocol::Evidence;
use super::session::FrameKey;

pub(super) struct Target {
    pub(super) frame: FrameKey,
    pub(super) refocus: bool,
}
pub(super) enum Attempt {
    First,
    Retry,
}
pub(super) enum Path {
    Inserted { reply: Reply },
    Handled,
    NoField { handled: bool },
}
pub(super) enum Reply {
    Ok,
    Uncertain,
}
pub(super) struct AttemptResult {
    pub(super) path: Path,
    pub(super) evidence: Option<Evidence>,
    pub(super) unobserved_input: bool,
    pub(super) all_frames_observed: bool,
}

pub(super) trait PageOps {
    fn resolve(&mut self, press: Option<Press>) -> Result<Target, PasteError>;
    fn attempt(
        &mut self,
        target: &Target,
        text: &str,
        attempt: Attempt,
    ) -> Result<AttemptResult, PasteError>;
}

pub(super) enum Next {
    Done(PasteOutcome),
    Retry,
}

pub(super) fn decide(result: &AttemptResult) -> Next {
    use PasteOutcome::{NoField, Pasted, Refused, Unconfirmed};

    if result.evidence.as_ref().is_some_and(|e| e.gone) {
        return Next::Done(Unconfirmed);
    }
    if let Path::NoField { handled } = result.path {
        return Next::Done(if handled {
            Pasted
        } else if result.unobserved_input {
            Unconfirmed
        } else {
            NoField
        });
    }
    let Some(evidence) = &result.evidence else {
        return Next::Done(Unconfirmed);
    };
    match result.path {
        Path::Inserted {
            reply: Reply::Uncertain,
        } => Next::Done(if evidence.changed {
            Pasted
        } else {
            Unconfirmed
        }),
        Path::Handled => Next::Done(if evidence.changed || evidence.focus_moved {
            Pasted
        } else if result.unobserved_input {
            Unconfirmed
        } else {
            Refused
        }),
        Path::Inserted { reply: Reply::Ok } => {
            if evidence.changed || evidence.input > 0 || evidence.focus_moved {
                Next::Done(Pasted)
            } else if result.unobserved_input {
                Next::Done(Unconfirmed)
            } else if evidence.before_input > 0 {
                Next::Done(Refused)
            } else if result.all_frames_observed {
                Next::Retry
            } else {
                // "Try again" is unsafe when an unobserved frame may have received text.
                Next::Done(Unconfirmed)
            }
        }
        Path::NoField { .. } => unreachable!(),
    }
}

pub(super) fn run(
    page: &mut impl PageOps,
    text: &str,
    press: Option<Press>,
) -> Result<PasteOutcome, PasteError> {
    let target = page.resolve(press)?;
    let first = page.attempt(&target, text, Attempt::First)?;
    match decide(&first) {
        Next::Done(outcome) => Ok(outcome),
        Next::Retry => {
            // The first prepare already touched the page, even if retry cannot start.
            let Ok(second) = page.attempt(&target, text, Attempt::Retry) else {
                return Ok(PasteOutcome::Unconfirmed);
            };
            Ok(match decide(&second) {
                Next::Done(outcome) => outcome,
                Next::Retry => PasteOutcome::NotLanded,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::session::SessionId;
    use super::*;
    use std::collections::VecDeque;

    fn result(path: Path, bits: u8) -> AttemptResult {
        AttemptResult {
            path,
            evidence: Some(Evidence {
                changed: bits & 1 != 0,
                before_input: u32::from(bits & 2 != 0),
                input: u32::from(bits & 4 != 0),
                focus_moved: bits & 8 != 0,
                gone: bits & 16 != 0,
            }),
            unobserved_input: bits & 32 != 0,
            all_frames_observed: bits & 64 == 0,
        }
    }

    fn assert_next(result: &AttemptResult, expected: Option<PasteOutcome>) {
        let actual = match decide(result) {
            Next::Done(outcome) => Some(outcome),
            Next::Retry => None,
        };
        assert_eq!(actual, expected);
    }

    #[test]
    fn tests_that_insert_decisions_cover_every_evidence_combination() {
        for bits in 0..128 {
            let expected = match bits {
                b if b & 16 != 0 => Some(PasteOutcome::Unconfirmed),
                b if b & (1 | 4 | 8) != 0 => Some(PasteOutcome::Pasted),
                b if b & 32 != 0 => Some(PasteOutcome::Unconfirmed),
                b if b & 2 != 0 => Some(PasteOutcome::Refused),
                b if b & 64 != 0 => Some(PasteOutcome::Unconfirmed),
                _ => None,
            };
            assert_next(&result(Path::Inserted { reply: Reply::Ok }, bits), expected);
        }
    }

    #[test]
    fn tests_that_uncertain_inserts_require_read_back_change_and_never_retry() {
        for bits in 0..128 {
            let expected = if bits & 16 == 0 && bits & 1 != 0 {
                PasteOutcome::Pasted
            } else {
                PasteOutcome::Unconfirmed
            };
            assert_next(
                &result(
                    Path::Inserted {
                        reply: Reply::Uncertain,
                    },
                    bits,
                ),
                Some(expected),
            );
        }
    }

    #[test]
    fn tests_that_handled_paste_never_inserts_or_retries() {
        for bits in 0..128 {
            let expected = match bits {
                b if b & 16 != 0 => PasteOutcome::Unconfirmed,
                b if b & (1 | 8) != 0 => PasteOutcome::Pasted,
                b if b & 32 != 0 => PasteOutcome::Unconfirmed,
                _ => PasteOutcome::Refused,
            };
            assert_next(&result(Path::Handled, bits), Some(expected));
        }
    }

    #[test]
    fn tests_that_confirmed_no_field_uses_the_handlers_result_unless_the_document_is_gone() {
        for handled in [false, true] {
            let expected = if handled {
                PasteOutcome::Pasted
            } else {
                PasteOutcome::NoField
            };
            for bits in 0..128 {
                assert_next(
                    &result(Path::NoField { handled }, bits),
                    Some(if bits & 16 != 0 || (!handled && bits & 32 != 0) {
                        PasteOutcome::Unconfirmed
                    } else {
                        expected
                    }),
                );
            }
            let mut r = result(Path::NoField { handled }, 0);
            r.evidence = None;
            assert_next(&r, Some(expected));
        }
    }

    #[test]
    fn tests_that_missing_read_back_never_claims_absence() {
        for path in [
            Path::Inserted { reply: Reply::Ok },
            Path::Inserted {
                reply: Reply::Uncertain,
            },
            Path::Handled,
        ] {
            let mut r = result(path, 0);
            r.evidence = None;
            assert_next(&r, Some(PasteOutcome::Unconfirmed));
        }
    }

    struct FakePage {
        results: VecDeque<Result<AttemptResult, PasteError>>,
        attempts: Vec<bool>,
        press: Option<Press>,
        unavailable: bool,
    }

    impl FakePage {
        fn new(results: impl IntoIterator<Item = Result<AttemptResult, PasteError>>) -> Self {
            Self {
                results: results.into_iter().collect(),
                attempts: Vec::new(),
                press: None,
                unavailable: false,
            }
        }
    }

    impl PageOps for FakePage {
        fn resolve(&mut self, press: Option<Press>) -> Result<Target, PasteError> {
            self.press = press;
            if self.unavailable {
                return Err(PasteError::Untouched("not connected"));
            }
            Ok(Target {
                frame: FrameKey {
                    session: SessionId("page".into()),
                    frame: "target".into(),
                },
                refocus: true,
            })
        }

        fn attempt(
            &mut self,
            target: &Target,
            text: &str,
            attempt: Attempt,
        ) -> Result<AttemptResult, PasteError> {
            assert_eq!(target.frame.session.0, "page");
            assert_eq!(target.frame.frame, "target");
            assert!(target.refocus);
            assert_eq!(text, "héllo");
            self.attempts.push(matches!(attempt, Attempt::Retry));
            self.results.pop_front().expect("unexpected extra attempt")
        }
    }

    #[test]
    fn tests_that_a_proven_miss_retries_once_on_the_same_target() {
        let mut page = FakePage::new([
            Ok(result(Path::Inserted { reply: Reply::Ok }, 0)),
            Ok(result(Path::Inserted { reply: Reply::Ok }, 1)),
        ]);
        let press = Some(Press {
            wall_ms: 100,
            x: 2,
            y: 3,
        });
        assert_eq!(
            run(&mut page, "héllo", press).unwrap(),
            PasteOutcome::Pasted
        );
        assert_eq!(page.press, press);
        assert_eq!(page.attempts, [false, true]);
    }

    #[test]
    fn tests_that_a_second_proven_miss_finishes_without_a_third_attempt() {
        let mut page =
            FakePage::new((0..2).map(|_| Ok(result(Path::Inserted { reply: Reply::Ok }, 0))));
        assert_eq!(
            run(&mut page, "héllo", None).unwrap(),
            PasteOutcome::NotLanded
        );
        assert_eq!(page.attempts, [false, true]);
    }

    #[test]
    fn tests_that_run_retries_only_with_the_complete_proof() {
        for bits in 1..128 {
            let r = result(Path::Inserted { reply: Reply::Ok }, bits);
            let Next::Done(expected) = decide(&r) else {
                panic!("incomplete proof retried")
            };
            let mut page = FakePage::new([Ok(r)]);
            assert_eq!(run(&mut page, "héllo", None).unwrap(), expected);
            assert_eq!(page.attempts, [false]);
        }
        for path in [
            Path::Handled,
            Path::NoField { handled: true },
            Path::NoField { handled: false },
            Path::Inserted {
                reply: Reply::Uncertain,
            },
        ] {
            let r = result(path, 0);
            let Next::Done(expected) = decide(&r) else {
                panic!("non-insert path retried")
            };
            let mut page = FakePage::new([Ok(r)]);
            assert_eq!(run(&mut page, "héllo", None).unwrap(), expected);
            assert_eq!(page.attempts, [false]);
        }
    }

    #[test]
    fn tests_that_only_an_untouched_first_attempt_can_request_fallback() {
        let mut page = FakePage::new([]);
        page.unavailable = true;
        assert!(matches!(
            run(&mut page, "héllo", None),
            Err(PasteError::Untouched("not connected"))
        ));
        assert!(page.attempts.is_empty());
        let mut page = FakePage::new([Err(PasteError::Untouched("not sent"))]);
        assert!(matches!(
            run(&mut page, "héllo", None),
            Err(PasteError::Untouched("not sent"))
        ));
        assert_eq!(page.attempts, [false]);
        let mut page = FakePage::new([
            Ok(result(Path::Inserted { reply: Reply::Ok }, 0)),
            Err(PasteError::Untouched("context lost")),
        ]);
        assert_eq!(
            run(&mut page, "héllo", None).unwrap(),
            PasteOutcome::Unconfirmed
        );
        assert_eq!(page.attempts, [false, true]);
    }
}
