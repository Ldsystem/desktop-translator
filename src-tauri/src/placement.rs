//! Direction-independent physical-pixel placement across mixed-scale monitors.

use crate::contracts::PhysicalRect;

/// Width and height in the coordinate space named by the caller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicalSize {
    pub width: f64,
    pub height: f64,
}

/// Point in global physical screen pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicalPoint {
    pub x: f64,
    pub y: f64,
}

/// Current physical work area and scale for one monitor.
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorWorkArea {
    pub id: String,
    pub work_area_physical_px: PhysicalRect,
    pub scale_factor: f64,
}

/// Fully resolved overlay position and size for a selected monitor.
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayPlacement {
    pub monitor_id: String,
    pub position_physical_px: PhysicalPoint,
    pub size_physical_px: PhysicalSize,
}

fn place_overlay(
    anchor: PhysicalRect,
    work_area: PhysicalRect,
    overlay: PhysicalSize,
    gap: f64,
) -> PhysicalPoint {
    let mut x = anchor.x + anchor.width + gap;
    let mut y = anchor.y + anchor.height + gap;
    let work_right = work_area.x + work_area.width;
    let work_bottom = work_area.y + work_area.height;

    if x + overlay.width > work_right {
        x = anchor.x - overlay.width - gap;
    }
    if y + overlay.height > work_bottom {
        y = anchor.y - overlay.height - gap;
    }

    let max_x = (work_right - overlay.width).max(work_area.x);
    let max_y = (work_bottom - overlay.height).max(work_area.y);

    PhysicalPoint {
        x: x.clamp(work_area.x, max_x),
        y: y.clamp(work_area.y, max_y),
    }
}

/// Centers a tray popover on the click. Prefers below the icon, flips above when
/// that would overflow, then clamps both axes to the work area.
pub fn place_tray_panel(
    anchor: PhysicalPoint,
    work_area: PhysicalRect,
    panel: PhysicalSize,
    gap: f64,
) -> PhysicalPoint {
    let work_right = work_area.x + work_area.width;
    let work_bottom = work_area.y + work_area.height;
    let max_x = (work_right - panel.width).max(work_area.x);
    let max_y = (work_bottom - panel.height).max(work_area.y);
    let x = (anchor.x - panel.width / 2.0).clamp(work_area.x, max_x);
    let below = anchor.y + gap;
    let y = if below + panel.height <= work_bottom {
        below
    } else {
        anchor.y - panel.height - gap
    };
    PhysicalPoint {
        x,
        y: y.clamp(work_area.y, max_y),
    }
}

/// Returns the final valid accessibility rectangle in provider-supplied reading order.
pub fn final_visible_line(rectangles: &[PhysicalRect]) -> Option<PhysicalRect> {
    rectangles
        .iter()
        .copied()
        .rfind(|rect| rect.width > 0.0 && rect.height > 0.0)
}

/// Selects the nearest monitor, scales logical dimensions, and clamps the overlay.
pub fn place_overlay_on_monitors(
    anchor: PhysicalRect,
    monitors: &[MonitorWorkArea],
    overlay_logical_size: PhysicalSize,
    logical_gap: f64,
) -> Option<OverlayPlacement> {
    if !overlay_logical_size.width.is_finite()
        || !overlay_logical_size.height.is_finite()
        || overlay_logical_size.width <= 0.0
        || overlay_logical_size.height <= 0.0
        || !logical_gap.is_finite()
        || logical_gap < 0.0
    {
        return None;
    }
    let center_x = anchor.x + anchor.width / 2.0;
    let center_y = anchor.y + anchor.height / 2.0;
    let monitor = monitors
        .iter()
        .filter(|monitor| {
            let work_area = monitor.work_area_physical_px;
            monitor.scale_factor.is_finite()
                && monitor.scale_factor > 0.0
                && work_area.x.is_finite()
                && work_area.y.is_finite()
                && work_area.width.is_finite()
                && work_area.height.is_finite()
                && work_area.width > 0.0
                && work_area.height > 0.0
        })
        .min_by(|left, right| {
            distance_to_rect(center_x, center_y, left.work_area_physical_px).total_cmp(
                &distance_to_rect(center_x, center_y, right.work_area_physical_px),
            )
        })?;
    let size_physical_px = PhysicalSize {
        width: overlay_logical_size.width * monitor.scale_factor,
        height: overlay_logical_size.height * monitor.scale_factor,
    };

    Some(OverlayPlacement {
        monitor_id: monitor.id.clone(),
        position_physical_px: place_overlay(
            anchor,
            monitor.work_area_physical_px,
            size_physical_px,
            logical_gap * monitor.scale_factor,
        ),
        size_physical_px,
    })
}

fn distance_to_rect(x: f64, y: f64, rect: PhysicalRect) -> f64 {
    let dx = if x < rect.x {
        rect.x - x
    } else if x > rect.x + rect.width {
        x - (rect.x + rect.width)
    } else {
        0.0
    };
    let dy = if y < rect.y {
        rect.y - y
    } else if y > rect.y + rect.height {
        y - (rect.y + rect.height)
    } else {
        0.0
    };
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use crate::contracts::PhysicalRect;

    use super::{
        final_visible_line, place_overlay, place_overlay_on_monitors, place_tray_panel,
        MonitorWorkArea, PhysicalPoint, PhysicalSize,
    };

    const WORK_AREA: PhysicalRect = PhysicalRect {
        x: -1000.0,
        y: 0.0,
        width: 1000.0,
        height: 800.0,
    };

    #[test]
    fn prefers_lower_right_of_anchor() {
        let point = place_overlay(
            PhysicalRect {
                x: -800.0,
                y: 200.0,
                width: 100.0,
                height: 20.0,
            },
            WORK_AREA,
            PhysicalSize {
                width: 120.0,
                height: 80.0,
            },
            8.0,
        );

        assert_eq!(
            point,
            PhysicalPoint {
                x: -692.0,
                y: 228.0
            }
        );
    }

    #[test]
    fn flips_and_clamps_near_work_area_edges() {
        let point = place_overlay(
            PhysicalRect {
                x: -40.0,
                y: 770.0,
                width: 30.0,
                height: 20.0,
            },
            WORK_AREA,
            PhysicalSize {
                width: 200.0,
                height: 160.0,
            },
            8.0,
        );

        assert_eq!(
            point,
            PhysicalPoint {
                x: -248.0,
                y: 602.0
            }
        );
    }

    #[test]
    fn uses_last_valid_multiline_rectangle() {
        let first = PhysicalRect {
            x: 10.0,
            y: 10.0,
            width: 100.0,
            height: 20.0,
        };
        let last = PhysicalRect {
            x: 10.0,
            y: 40.0,
            width: 40.0,
            height: 20.0,
        };

        assert_eq!(final_visible_line(&[first, last]), Some(last));
    }

    #[test]
    fn selects_correct_mixed_scale_monitor_and_scales_overlay() {
        let monitors = [
            MonitorWorkArea {
                id: "left".into(),
                work_area_physical_px: PhysicalRect {
                    x: -1920.0,
                    y: 0.0,
                    width: 1920.0,
                    height: 1080.0,
                },
                scale_factor: 1.0,
            },
            MonitorWorkArea {
                id: "retina".into(),
                work_area_physical_px: PhysicalRect {
                    x: 0.0,
                    y: 0.0,
                    width: 3024.0,
                    height: 1890.0,
                },
                scale_factor: 2.0,
            },
        ];
        let placement = place_overlay_on_monitors(
            PhysicalRect {
                x: 2900.0,
                y: 1800.0,
                width: 80.0,
                height: 40.0,
            },
            &monitors,
            PhysicalSize {
                width: 200.0,
                height: 120.0,
            },
            8.0,
        )
        .expect("monitor must be selected");

        assert_eq!(placement.monitor_id, "retina");
        assert_eq!(
            placement.size_physical_px,
            PhysicalSize {
                width: 400.0,
                height: 240.0
            }
        );
        assert!(placement.position_physical_px.x >= 0.0);
        assert!(placement.position_physical_px.x + 400.0 <= 3024.0);
        assert!(placement.position_physical_px.y + 240.0 <= 1890.0);
    }

    #[test]
    fn places_rtl_final_line_geometry_without_direction_assumptions() {
        let rtl_lines = [
            PhysicalRect {
                x: 400.0,
                y: 100.0,
                width: 180.0,
                height: 24.0,
            },
            PhysicalRect {
                x: 120.0,
                y: 132.0,
                width: 90.0,
                height: 24.0,
            },
        ];
        let anchor = final_visible_line(&rtl_lines).expect("final RTL line exists");
        let point = place_overlay(
            anchor,
            PhysicalRect {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            },
            PhysicalSize {
                width: 120.0,
                height: 80.0,
            },
            8.0,
        );

        assert_eq!(point.x, 218.0);
        assert_eq!(point.y, 164.0);
    }

    #[test]
    fn recomputes_after_monitor_topology_scale_change() {
        let anchor = PhysicalRect {
            x: 500.0,
            y: 400.0,
            width: 80.0,
            height: 24.0,
        };
        let logical_size = PhysicalSize {
            width: 200.0,
            height: 120.0,
        };
        let initial = place_overlay_on_monitors(
            anchor,
            &[MonitorWorkArea {
                id: "main".into(),
                work_area_physical_px: PhysicalRect {
                    x: 0.0,
                    y: 0.0,
                    width: 1920.0,
                    height: 1080.0,
                },
                scale_factor: 1.0,
            }],
            logical_size,
            8.0,
        )
        .expect("initial topology is valid");
        let changed = place_overlay_on_monitors(
            anchor,
            &[MonitorWorkArea {
                id: "main".into(),
                work_area_physical_px: PhysicalRect {
                    x: 0.0,
                    y: 0.0,
                    width: 3024.0,
                    height: 1890.0,
                },
                scale_factor: 2.0,
            }],
            logical_size,
            8.0,
        )
        .expect("changed topology is valid");

        assert_eq!(initial.size_physical_px.width, 200.0);
        assert_eq!(changed.size_physical_px.width, 400.0);
        assert_ne!(initial.position_physical_px, changed.position_physical_px);
    }

    #[test]
    fn rejects_invalid_topology_or_logical_size() {
        let invalid = place_overlay_on_monitors(
            PhysicalRect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            &[MonitorWorkArea {
                id: "broken".into(),
                work_area_physical_px: PhysicalRect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 100.0,
                },
                scale_factor: 1.0,
            }],
            PhysicalSize {
                width: f64::NAN,
                height: 10.0,
            },
            8.0,
        );

        assert_eq!(invalid, None);
    }

    #[test]
    fn tray_panel_opens_below_when_the_work_area_has_room() {
        let point = place_tray_panel(
            PhysicalPoint { x: 200.0, y: 28.0 },
            PhysicalRect {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            },
            PhysicalSize {
                width: 400.0,
                height: 470.0,
            },
            6.0,
        );

        assert_eq!(point, PhysicalPoint { x: 0.0, y: 34.0 });
    }

    #[test]
    fn tray_panel_flips_above_and_clamps_to_the_work_area_near_the_bottom_right() {
        let point = place_tray_panel(
            PhysicalPoint {
                x: 1900.0,
                y: 1064.0,
            },
            PhysicalRect {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1040.0,
            },
            PhysicalSize {
                width: 400.0,
                height: 470.0,
            },
            6.0,
        );

        assert_eq!(
            point,
            PhysicalPoint {
                x: 1520.0,
                y: 570.0
            }
        );
        assert!(point.x + 400.0 <= 1920.0);
        assert!(point.y + 470.0 <= 1040.0);
        assert!(point.y + 470.0 + 6.0 <= 1064.0);
    }
}
