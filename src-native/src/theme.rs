use std::cell::Cell;

#[derive(Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    Default,
    Ocean,
    Forest,
    Violet,
    Amber,
}

impl Theme {
    pub const ALL: [Self; 5] = [Self::Default, Self::Ocean, Self::Forest, Self::Violet, Self::Amber];
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "经典 · 默认",
            Self::Ocean => "深海蓝",
            Self::Forest => "松林绿",
            Self::Violet => "暮光紫",
            Self::Amber => "暖砂棕",
        }
    }
    pub fn color(self, color: u32) -> u32 {
        if self == Self::Default { return color; }
        // Body, card, inset panel, border, track, badge, accent, 7d, 5h.
        let p = match self {
            Self::Ocean => [0x0b1726, 0x102239, 0x152c45, 0x2b4964, 0x3a526a, 0x153f55, 0x7edcf1, 0xffa577, 0x75baff],
            Self::Forest => [0x101c19, 0x172923, 0x1c332b, 0x365346, 0x425d50, 0x22483b, 0x9de0b2, 0xf3be78, 0x8ecdc3],
            Self::Violet => [0x191321, 0x251d32, 0x30263e, 0x51405f, 0x5b4b69, 0x443054, 0xd9b4f5, 0xffa5bd, 0xbba3ff],
            Self::Amber => [0x211a16, 0x30251f, 0x3a2e25, 0x5a4739, 0x685343, 0x503e29, 0xf2d08f, 0xf4a67e, 0xd9c488],
            Self::Default => unreachable!(),
        };
        let rgb = match color & 0xffffff {
            0x080a0e | 0x020203 => p[0],
            0x101d2e | 0x25313a | 0x242a35 => p[1],
            0x142234 | 0x121820 => p[2],
            0x233247 | 0x243447 | 0x252e39 | 0x272e39 | 0x282f3a | 0x293c4c => p[3],
            0x515761 | 0x343842 | 0x333944 | 0x343b45 | 0x444b58 | 0x576271 => p[4],
            0x103b3b | 0x103c38 | 0x124b3e | 0x17614d | 0x28514d => p[5],
            0x58dcb1 | 0x74f2b8 | 0x82f5cd | 0x61e0bc | 0x65e0a1 | 0x78edb1 => p[6],
            0xff6661 | 0xff6868 | 0xff7773 | 0xff8c87 => p[7],
            0xbc70f4 | 0xbd80f8 | 0xc99bff => p[8],
            _ => return color,
        };
        (color & 0xff000000) | rgb
    }
}

thread_local! { static ACTIVE: Cell<Theme> = const { Cell::new(Theme::Default) }; }
pub fn color(value: u32) -> u32 { ACTIVE.with(|theme| theme.get().color(value)) }
pub struct Scope(Theme);
pub fn enter(theme: Theme) -> Scope { Scope(ACTIVE.with(|active| active.replace(theme))) }
impl Drop for Scope {
    fn drop(&mut self) { ACTIVE.with(|active| active.set(self.0)); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_themes_round_trip_and_preserve_task_state_colors() {
        for theme in Theme::ALL {
            let loaded: Theme = serde_json::from_value(serde_json::to_value(theme).unwrap()).unwrap();
            assert!(loaded == theme);
            for color in [0xff626d7a, 0xff4f9dff, 0xff9bea58, 0xffffab62] {
                assert_eq!(theme.color(color), color);
            }
            assert_eq!(theme.color(0xdf242a35) >> 24, 0xdf);
        }
        assert_eq!(Theme::Default.color(0xff080a0e), 0xff080a0e);
    }
    #[test]
    fn nested_paint_scopes_restore_the_previous_theme() {
        let original = color(0xff080a0e);
        {
            let _ocean = enter(Theme::Ocean);
            assert_eq!(color(0xff080a0e), Theme::Ocean.color(0xff080a0e));
            {
                let _forest = enter(Theme::Forest);
                assert_eq!(color(0xff080a0e), Theme::Forest.color(0xff080a0e));
            }
            assert_eq!(color(0xff080a0e), Theme::Ocean.color(0xff080a0e));
        }
        assert_eq!(color(0xff080a0e), original);
    }
}
