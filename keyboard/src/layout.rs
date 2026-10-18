use log::error;

use crate::bt::{Key, KeyMod, MediaKey, MouseKey};
use crate::bt::ble_mouse_keyboard::BleMouseKeyboard;

macro_rules! k {
    ($key:ident) => {
        LayoutKey::Key(Key::$key)
    };
}

macro_rules! m {
    ($key:ident) => {
        LayoutKey::Mod(KeyMod::$key)
    };
}

macro_rules! km {
    ($key:ident, $modif:ident) => {
        LayoutKey::KeyWithMod(Key::$key, KeyMod::$modif)
    };
}

pub struct Layout<const W: usize, const H: usize> {
    name: &'static str,
    map: [[LayoutKey; W]; H],
    fn_coord: Option<(usize, usize)>
}

impl<const W: usize, const H: usize> Layout<W, H> {
    const fn new(name: &'static str, map: [[LayoutKey; W]; H]) -> Self {
        let mut fn_coord = None;

        let mut x = 0;
        'outer: while x < W {
            let mut y = 0;
            while y < H {
                if let &LayoutKey::Fn { .. } =  &map[y][x] {
                    fn_coord = Some((x, y));
                    break 'outer;
                }
                y += 1;
            }
            x += 1;
        }

        Self {
            name,
            map,
            fn_coord
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
 
    pub fn get(&self, x: usize, y: usize) -> &LayoutKey {
        &self.map[y][x]
    }

    pub fn fn_layout_key(&self) -> Option<&LayoutKey> {
        self.fn_coord.map_or(None, |fn_coord| Some(&self.map[fn_coord.1][fn_coord.0]))
    }

    pub fn fn_coords(&self) -> Option<(usize, usize)> {
        self.fn_coord
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum LayoutKey {
    Key(Key),
    KeyWithMod(Key, KeyMod),
    Mod(KeyMod),
    #[allow(unused)]
    Media(MediaKey),
    MouseKey(MouseKey),
    MouseTravel {
        dx: i8,
        dy: i8
    },
    MouseScroll {
        dy: i8
    },
    Fn {
        temporary_switch: usize,
        permanent_switch: usize,
    },
    None
}

impl LayoutKey {
    pub const fn new_fn(temporary_switch: usize, permanent_switch: usize) -> Self {
        LayoutKey::Fn {
            temporary_switch,
            permanent_switch
        }
    }

    #[allow(unused)]
    pub fn is_fn(&self) -> bool {
        if let LayoutKey::Fn { .. } = self {
            true
        } else {
            false
        }
    }

    /// Adds the held key to the reports.
    pub fn press(&self, keyboard: &mut BleMouseKeyboard) {
        match self {
            LayoutKey::Key(key) => keyboard.press_key(*key),
            LayoutKey::KeyWithMod(key, key_mod) => {
                keyboard.press_key_mod(*key_mod);
                keyboard.press_key(*key);
            },
            LayoutKey::Mod(key_mod) => keyboard.press_key_mod(*key_mod),
            LayoutKey::Media(media_key) => keyboard.press_media_key(*media_key),
            LayoutKey::MouseKey(mouse_key) => keyboard.press_mouse(*mouse_key),
            LayoutKey::MouseTravel { dx, dy } => keyboard.travel_mouse_cursor(*dx, *dy),
            LayoutKey::MouseScroll { dy } => keyboard.travel_mouse_wheel(*dy),
            LayoutKey::Fn { .. } | LayoutKey::None => { }
        }
    }
}

pub struct Layouts<const W: usize, const H: usize, const C: usize> {
    values: [Layout<W, H>; C]
}

impl<const W: usize, const H: usize, const C: usize> Layouts<W, H, C> {
    const fn new(values: [Layout<W, H>; C]) -> Self {
        Self {
            values
        }
    }
    
    pub(crate) fn get(&self, idx: usize) -> &Layout<W, H> {
        if idx >= C {
            error!("idx out of bounds, returning first layout.");
            return &self.values[0];
        }

        &self.values[idx]
    }
}

pub const DEFAULT: Layouts<10, 5, 3> = Layouts::new([
    Layout::new("MAIN", [
        [k!(Escape),   km!(Num1, LeftShift), km!(Num2, LeftShift),    km!(Num3, LeftShift), km!(Num7, LeftShift), k!(KpAsterisk), k!(LeftBracket), k!(RightBracket), k!(Minus),     k!(Equal)],
        [k!(Q),        k!(W),                k!(E),                   k!(R),                k!(T),                k!(Y),          k!(U),           k!(I),            k!(O),         k!(P)],
        [k!(A),        k!(S),                k!(D),                   k!(F),                k!(G),                k!(H),          k!(J),           k!(K),            k!(L),         k!(Semicolon)],
        [k!(Z),        k!(X),                k!(C),                   k!(V),                k!(B),                k!(N),          k!(M),           k!(Comma),        k!(Dot),       k!(Slash)],
        [m!(LeftCtrl), m!(LeftShift),        LayoutKey::new_fn(1, 2), m!(LeftAlt),          k!(Backspace),        k!(Space),      k!(Enter),       k!(Apostrophe),   k!(Backslash), k!(Tab)],
    ]),
    Layout::new("FUNCTION", [
        [k!(Grave),       k!(F1),          k!(F2),          k!(F3),      k!(F4),     k!(F5),    k!(LeftBracket), k!(RightBracket), km!(Num9, LeftShift), km!(Num0, LeftShift)],
        [LayoutKey::None, k!(F6),          k!(F7),          k!(F8),      k!(F9),     k!(F10),   k!(Num7),        k!(Num8),         k!(Num9),             k!(KpAsterisk)],
        [k!(CapsLock),    k!(F11),         k!(F12),         k!(Up),      k!(Up),     k!(Up),    k!(Num4),        k!(Num5),         k!(Num6),             k!(KpPlus)],
        [LayoutKey::None, LayoutKey::None, LayoutKey::None, k!(Left),    k!(Down),   k!(Right), k!(Num1),        k!(Num2),         k!(Num3),             k!(KpDot)],
        [m!(LeftCtrl),    m!(LeftShift),   LayoutKey::None, m!(LeftAlt), k!(Delete), k!(Space), k!(Num0),        k!(Minus),        k!(Slash),            k!(KpEnter)],
    ]),
    Layout::new("GAME", [
        [k!(Escape),   k!(Num1),      k!(Num2),                k!(Num3),    k!(Num4),      k!(Num5),  k!(Num6),  k!(RightBracket),                          k!(Minus),                                k!(Equal)],
        [k!(Q),        k!(W),         k!(E),                   k!(R),       k!(T),         k!(Y),     k!(U),     k!(I),                                     k!(O),                                    LayoutKey::MouseScroll { dy: 10 }],
        [k!(A),        k!(S),         k!(D),                   k!(F),       k!(G),         k!(H),     k!(J),     k!(K),                                     k!(L),                                    LayoutKey::MouseScroll { dy: -10 }],
        [k!(Z),        k!(X),         k!(C),                   k!(V),       k!(B),         k!(N),     k!(M),     LayoutKey::MouseKey(MouseKey::Left),       LayoutKey::MouseTravel { dx: 0, dy:-10 }, LayoutKey::MouseKey(MouseKey::Right)],
        [m!(LeftCtrl), m!(LeftShift), LayoutKey::new_fn(1, 0), m!(LeftAlt), k!(Backspace), k!(Space), k!(Enter), LayoutKey::MouseTravel { dx: -10, dy: 0 }, LayoutKey::MouseTravel { dx: 0, dy: 10 }, LayoutKey::MouseTravel { dx: 10, dy: 0 }],
    ])
]);

