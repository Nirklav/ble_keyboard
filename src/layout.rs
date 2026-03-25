use crate::bt::{Key, KeyMod, MediaKey, MouseKey};

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
    map: [[LayoutKey; W]; H]
}

pub enum LayoutKey {
    Key(Key),
    KeyWithMod(Key, KeyMod),
    Mod(KeyMod),
    Media(MediaKey),
    MouseKey(MouseKey),
    MouseTravel {
        dx: i8,
        dy: i8
    },
    MouseScroll {
        dy: i8
    },
    Fn,
    None
}

struct Layouts;

impl Layouts {
    pub const DEFAULT: Layout<10, 5> = Layout { 
        map: [
            [k!(Escape),   km!(Num1, LeftShift), km!(Num2, LeftShift), km!(Num3, LeftShift), km!(Num7, LeftShift), k!(KpAsterisk), k!(LeftBracket), k!(RightBracket), k!(Minus),     k!(Equal)],
            [k!(Q),        k!(W),                k!(E),                k!(R),                k!(T),                k!(Y),          k!(U),           k!(I),            k!(O),         k!(P)],
            [k!(A),        k!(S),                k!(D),                k!(F),                k!(G),                k!(H),          k!(J),           k!(K),            k!(L),         k!(Semicolon)],
            [k!(Z),        k!(X),                k!(C),                k!(V),                k!(B),                k!(N),          k!(M),           k!(Comma),        k!(Dot),       k!(Slash)],
            [m!(LeftCtrl), m!(LeftShift),        LayoutKey::Fn,        m!(LeftAlt),          k!(Backspace),        k!(Space),      k!(Enter),       k!(Apostrophe),   k!(Backslash), k!(Tab)],
        ]
    };


    pub const FUNCTION: Layout<10, 5> = Layout { 
        map: [
            [k!(Grave),       k!(F1),          k!(F2),          k!(F3),      k!(F4),     k!(F5),    k!(LeftBracket), k!(RightBracket), km!(Num9, LeftShift), km!(Num0, LeftShift)],
            [LayoutKey::None, k!(F6),          k!(F7),          k!(F8),      k!(F9),     k!(F10),   k!(Num7),        k!(Num8),         k!(Num9),             k!(KpAsterisk)],
            [k!(CapsLock),    k!(F11),         k!(F12),         k!(Up),      k!(Up),     k!(Up),    k!(Num4),        k!(Num5),         k!(Num6),             k!(KpPlus)],
            [LayoutKey::None, LayoutKey::None, LayoutKey::None, k!(Left),    k!(Down),   k!(Right), k!(Num1),        k!(Num2),         k!(Num3),             k!(KpDot)],
            [m!(LeftCtrl),    m!(LeftShift),   LayoutKey::Fn,   m!(LeftAlt), k!(Delete), k!(Space), k!(Num0),        k!(Minus),        k!(Slash),            k!(KpEnter)],
        ]
    };

    pub const GAME: Layout<10, 5> = Layout { 
        map: [
            [k!(Escape),   k!(Num1),      k!(Num2),      k!(Num3),    k!(Num4),      k!(Num5),  k!(Num6),  k!(RightBracket),                          k!(Minus),                                k!(Equal)],
            [k!(Q),        k!(W),         k!(E),         k!(R),       k!(T),         k!(Y),     k!(U),     k!(I),                                     k!(O),                                    LayoutKey::MouseScroll { dy: 10 }],
            [k!(A),        k!(S),         k!(D),         k!(F),       k!(G),         k!(H),     k!(J),     k!(K),                                     k!(L),                                    LayoutKey::MouseScroll { dy: -10 }],
            [k!(Z),        k!(X),         k!(C),         k!(V),       k!(B),         k!(N),     k!(M),     LayoutKey::MouseKey(MouseKey::Left),       LayoutKey::MouseTravel { dx: 0, dy:-10 }, LayoutKey::MouseKey(MouseKey::Right)],
            [m!(LeftCtrl), m!(LeftShift), LayoutKey::Fn, m!(LeftAlt), k!(Backspace), k!(Space), k!(Enter), LayoutKey::MouseTravel { dx: -10, dy: 0 }, LayoutKey::MouseTravel { dx: 0, dy: 10 }, LayoutKey::MouseTravel { dx: 10, dy: 0 }],
        ]
    };
}

