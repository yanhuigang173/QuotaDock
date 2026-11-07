use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Edge {
    #[default]
    None,
    Top,
    Bottom,
    ScreenBottom,
    Left,
    Right,
}

impl Edge {
    pub fn vertical(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

#[derive(Clone, Copy)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[cfg(test)]
pub fn detect(rect: Bounds, work: Bounds, current: Edge, dpi: f32) -> Edge {
    detect_with_screen(rect, work, work, current, dpi)
}

pub fn detect_with_screen(rect: Bounds, work: Bounds, screen: Bounds, current: Edge, dpi: f32) -> Edge {
    let distance = |a: i32, b: i32| a.abs_diff(b).min(i32::MAX as u32) as i32;
    let mut gaps = vec![
        // Dragging a wide HUD to a side commonly moves part of it offscreen.
        // Crossing that edge is contact, not an increasing separation from it.
        (Edge::Left, (rect.left - work.left).max(0)),
        (Edge::Right, (work.right - rect.right).max(0)),
        (Edge::Top, (rect.top - work.top).max(0)),
        // Keep the two bottom rails distinct when crossing the taskbar region.
        (Edge::Bottom, distance(work.bottom, rect.bottom)),
    ];
    if screen.bottom > work.bottom {
        gaps.push((Edge::ScreenBottom, distance(screen.bottom, rect.bottom)));
    }
    let enter = (24.0 * dpi).round() as i32;
    let leave = (48.0 * dpi).round() as i32;
    let switch_margin = (8.0 * dpi).round() as i32;
    let nearest = gaps.iter().min_by_key(|(_, gap)| *gap).copied();
    // Direct side contact should turn a horizontal strip into a vertical strip,
    // even at a corner where its old top/bottom rail is also still close.
    let side_contact = nearest.is_some_and(|(edge, gap)| edge.vertical() && gap == 0 && !current.vertical());
    if let Some((_, current_gap)) = gaps.iter().find(|(edge, _)| *edge == current) {
        if !side_contact && *current_gap <= leave && nearest.is_some_and(|(_, gap)| gap > enter || gap + switch_margin >= *current_gap) {
            return current;
        }
    }
    nearest
        .filter(|(_, gap)| *gap <= enter)
        .map(|(edge, _)| edge)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    const WORK: Bounds = Bounds {
        left: -1920,
        top: 0,
        right: 0,
        bottom: 1040,
    };
    #[test]
    fn all_edges_and_center_on_a_negative_origin_monitor() {
        for (rect, edge) in [
            (
                Bounds {
                    left: -1200,
                    top: 8,
                    right: -850,
                    bottom: 68,
                },
                Edge::Top,
            ),
            (
                Bounds {
                    left: -1200,
                    top: 970,
                    right: -850,
                    bottom: 1030,
                },
                Edge::Bottom,
            ),
            (
                Bounds {
                    left: -1910,
                    top: 300,
                    right: -1560,
                    bottom: 360,
                },
                Edge::Left,
            ),
            (
                Bounds {
                    left: -355,
                    top: 300,
                    right: -5,
                    bottom: 360,
                },
                Edge::Right,
            ),
            (
                Bounds {
                    left: -1200,
                    top: 400,
                    right: -850,
                    bottom: 460,
                },
                Edge::None,
            ),
        ] {
            assert_eq!(detect(rect, WORK, Edge::None, 1.0), edge);
        }
    }
    #[test]
    fn hysteresis_avoids_repeated_docking_near_the_threshold() {
        let rect = Bounds {
            left: -1890,
            top: 300,
            right: -1870,
            bottom: 650,
        };
        assert_eq!(detect(rect, WORK, Edge::None, 1.0), Edge::None);
        assert_eq!(detect(rect, WORK, Edge::Left, 1.0), Edge::Left);
        assert_eq!(detect(rect, WORK, Edge::None, 1.5), Edge::Left);
    }
}
