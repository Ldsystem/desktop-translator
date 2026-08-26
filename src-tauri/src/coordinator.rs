//! Pure newest-request-wins lifecycle for selection and translation overlays.

use crate::contracts::{AppError, SelectionSnapshot, TranslationResult};

/// Complete native coordinator state; content is retained only by visible variants.
#[derive(Debug, Clone, PartialEq)]
pub enum OverlayState {
    Disabled,
    Idle {
        latest_selection_id: u64,
        generation: u64,
    },
    PointerDown {
        latest_selection_id: u64,
        generation: u64,
    },
    ResolvingSelection {
        latest_selection_id: u64,
        generation: u64,
        request_id: u64,
    },
    ButtonVisible {
        selection: SelectionSnapshot,
        generation: u64,
    },
    Translating {
        selection: SelectionSnapshot,
        generation: u64,
    },
    ResultVisible {
        selection: SelectionSnapshot,
        result: TranslationResult,
        generation: u64,
    },
    ErrorVisible {
        selection: SelectionSnapshot,
        error: AppError,
        generation: u64,
    },
}

/// Input events accepted by the pure coordinator reducer.
#[derive(Debug, Clone, PartialEq)]
pub enum CoordinatorEvent {
    Enable,
    Disable,
    PointerDown,
    PointerUp {
        request_id: u64,
    },
    SelectionResolved {
        request_id: u64,
        selection: SelectionSnapshot,
    },
    SelectionRejected {
        request_id: u64,
    },
    Translate,
    TranslationResolved(TranslationResult),
    TranslationFailed {
        selection_id: u64,
        error: AppError,
    },
    Dismiss,
}

impl OverlayState {
    /// Returns the enabled startup state.
    pub fn initial() -> Self {
        Self::Idle {
            latest_selection_id: 0,
            generation: 0,
        }
    }

    /// Applies one event while rejecting stale selection and translation responses.
    pub fn reduce(self, event: CoordinatorEvent) -> Self {
        if event == CoordinatorEvent::Disable {
            return Self::Disabled;
        }

        if self == Self::Disabled {
            return match event {
                CoordinatorEvent::Enable => Self::initial(),
                _ => self,
            };
        }

        let latest_id = self.latest_selection_id();
        let generation = self.generation();

        match event {
            CoordinatorEvent::Enable => self,
            CoordinatorEvent::Disable => Self::Disabled,
            CoordinatorEvent::PointerDown => Self::PointerDown {
                latest_selection_id: latest_id,
                generation: generation + 1,
            },
            CoordinatorEvent::PointerUp { request_id } => Self::ResolvingSelection {
                latest_selection_id: latest_id,
                generation: generation + 1,
                request_id,
            },
            CoordinatorEvent::SelectionResolved {
                request_id,
                selection,
            } => match self {
                Self::ResolvingSelection {
                    latest_selection_id,
                    generation,
                    request_id: pending_request_id,
                } if request_id == pending_request_id && selection.id > latest_selection_id => {
                    Self::ButtonVisible {
                        selection,
                        generation,
                    }
                }
                _ => self,
            },
            CoordinatorEvent::SelectionRejected { request_id } => match self {
                Self::ResolvingSelection {
                    latest_selection_id,
                    generation,
                    request_id: pending_request_id,
                } if request_id == pending_request_id => Self::Idle {
                    latest_selection_id,
                    generation,
                },
                _ => self,
            },
            CoordinatorEvent::Dismiss => Self::Idle {
                latest_selection_id: latest_id,
                generation: generation + 1,
            },
            CoordinatorEvent::Translate => match self {
                Self::ButtonVisible {
                    selection,
                    generation,
                }
                | Self::ResultVisible {
                    selection,
                    generation,
                    ..
                }
                | Self::ErrorVisible {
                    selection,
                    generation,
                    ..
                } => Self::Translating {
                    selection,
                    generation,
                },
                _ => self,
            },
            CoordinatorEvent::TranslationResolved(result) => match self {
                Self::Translating {
                    selection,
                    generation,
                } if result.selection_id == selection.id => Self::ResultVisible {
                    selection,
                    result,
                    generation,
                },
                _ => self,
            },
            CoordinatorEvent::TranslationFailed {
                selection_id,
                error,
            } => match self {
                Self::Translating {
                    selection,
                    generation,
                } if selection_id == selection.id => Self::ErrorVisible {
                    selection,
                    error,
                    generation,
                },
                _ => self,
            },
        }
    }

    /// Returns the newest selection identifier observed by this state.
    pub fn latest_selection_id(&self) -> u64 {
        match self {
            Self::Disabled => 0,
            Self::Idle {
                latest_selection_id,
                ..
            }
            | Self::PointerDown {
                latest_selection_id,
                ..
            }
            | Self::ResolvingSelection {
                latest_selection_id,
                ..
            } => *latest_selection_id,
            Self::ButtonVisible { selection, .. }
            | Self::Translating { selection, .. }
            | Self::ResultVisible { selection, .. }
            | Self::ErrorVisible { selection, .. } => selection.id,
        }
    }

    /// Returns the invalidation generation used to correlate asynchronous work.
    pub fn generation(&self) -> u64 {
        match self {
            Self::Disabled => 0,
            Self::Idle { generation, .. }
            | Self::PointerDown { generation, .. }
            | Self::ResolvingSelection { generation, .. }
            | Self::ButtonVisible { generation, .. }
            | Self::Translating { generation, .. }
            | Self::ResultVisible { generation, .. }
            | Self::ErrorVisible { generation, .. } => *generation,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::contracts::{PhysicalRect, SelectionSnapshot};

    use super::{CoordinatorEvent, OverlayState};

    fn selection(id: u64) -> SelectionSnapshot {
        let bounds = PhysicalRect {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 15.0,
        };
        SelectionSnapshot {
            id,
            text: "selected".into(),
            example_sentence: None,
            source_application_id: None,
            bounds_physical_px: vec![bounds],
            anchor_physical_px: bounds,
            captured_at_epoch_ms: 1,
        }
    }

    #[test]
    fn newest_selection_wins() {
        let visible = OverlayState::initial()
            .reduce(CoordinatorEvent::PointerUp { request_id: 1 })
            .reduce(CoordinatorEvent::SelectionResolved {
                request_id: 1,
                selection: selection(2),
            });
        let unchanged = visible.clone().reduce(CoordinatorEvent::SelectionResolved {
            request_id: 1,
            selection: selection(1),
        });

        assert_eq!(unchanged, visible);
    }

    #[test]
    fn pointer_down_hides_visible_content() {
        let state = OverlayState::ButtonVisible {
            selection: selection(2),
            generation: 1,
        }
        .reduce(CoordinatorEvent::PointerDown);

        assert_eq!(
            state,
            OverlayState::PointerDown {
                latest_selection_id: 2,
                generation: 2,
            }
        );
    }

    #[test]
    fn a_rejected_gesture_does_not_block_the_next_selection_request() {
        let after_rejection = OverlayState::initial()
            .reduce(CoordinatorEvent::PointerDown)
            .reduce(CoordinatorEvent::PointerUp { request_id: 2 })
            .reduce(CoordinatorEvent::SelectionRejected { request_id: 2 });

        let next = after_rejection
            .reduce(CoordinatorEvent::PointerDown)
            .reduce(CoordinatorEvent::PointerUp { request_id: 3 });

        assert!(matches!(
            next,
            OverlayState::ResolvingSelection { request_id: 3, .. }
        ));
    }

    #[test]
    fn disable_drops_in_memory_selection() {
        let state = OverlayState::ButtonVisible {
            selection: selection(2),
            generation: 1,
        }
        .reduce(CoordinatorEvent::Disable);

        assert_eq!(state, OverlayState::Disabled);
    }

    #[test]
    fn stale_selection_cannot_revive_after_dismiss() {
        let resolving =
            OverlayState::initial().reduce(CoordinatorEvent::PointerUp { request_id: 1 });
        let dismissed = resolving.reduce(CoordinatorEvent::Dismiss);
        let late = dismissed
            .clone()
            .reduce(CoordinatorEvent::SelectionResolved {
                request_id: 1,
                selection: selection(2),
            });

        assert_eq!(late, dismissed);
    }

    #[test]
    fn a_new_selection_request_is_accepted_after_dismiss() {
        let visible = OverlayState::ButtonVisible {
            selection: selection(2),
            generation: 1,
        };
        let next = visible
            .reduce(CoordinatorEvent::Dismiss)
            .reduce(CoordinatorEvent::PointerDown)
            .reduce(CoordinatorEvent::PointerUp { request_id: 2 });

        assert!(matches!(
            next,
            OverlayState::ResolvingSelection { request_id: 2, .. }
        ));
    }

    #[test]
    fn pending_selection_work_is_coalesced_to_newest_request() {
        let first = OverlayState::initial().reduce(CoordinatorEvent::PointerUp { request_id: 1 });
        let newest = first.reduce(CoordinatorEvent::PointerUp { request_id: 2 });
        let stale = newest.clone().reduce(CoordinatorEvent::SelectionResolved {
            request_id: 1,
            selection: selection(2),
        });

        assert_eq!(stale, newest);
        assert!(matches!(
            newest,
            OverlayState::ResolvingSelection {
                request_id: 2,
                generation: 2,
                ..
            }
        ));
    }
}
