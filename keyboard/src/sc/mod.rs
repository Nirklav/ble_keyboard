use core::fmt::Debug;

use crate::key_matrix::KeyChange;
use crate::layout::LayoutKey;
use crate::services::Services;

pub mod switch_layout;
pub mod unpair;

pub struct SpecialCombinationBuilder {
    combination: Vec<ScKey>
}

impl SpecialCombinationBuilder {
    pub fn new() -> Self {
        Self {
            combination: Vec::new()
        }
    }

    pub fn by_coord(mut self, x: u8, y: u8, required_count: Option<u8>) -> Self {
        let sc_key = ScKey::new(ScKeyIdent::Coord { x, y }, required_count);
        self.combination.push(sc_key);
        self
    }

    #[allow(unused)]
    pub fn by_name(mut self, key: LayoutKey, required_count: Option<u8>) -> Self {
        let sc_key = ScKey::new(ScKeyIdent::Name(key), required_count);
        self.combination.push(sc_key);
        self
    }

    pub fn build<A: SpecialCombinationAction<W, H, C> + 'static, const W: usize, const H: usize, const C: usize>(self, action: A) -> SpecialCombination<W, H, C> {
        SpecialCombination {
            action: Box::new(action),
            combination: self.combination,
            invalid: false
        }
    }
}

pub trait SpecialCombinationAction<const W: usize, const H: usize, const C: usize> : Debug {
    fn on_progress(&mut self, _combinations: &Vec<ScKey>, _svc: &mut Services<W, H, C>)  -> anyhow::Result<()> {
        Ok(())
    }

    fn execute(&mut self, svc: &mut Services<W, H, C>)  -> anyhow::Result<()>;
}

#[derive(Debug)]
pub struct SpecialCombination<const W: usize, const H: usize, const C: usize> {
    action: Box<dyn SpecialCombinationAction<W, H, C>>,
    combination: Vec<ScKey>,
    invalid: bool
}

impl<const W: usize, const H: usize, const C: usize> SpecialCombination<W, H, C> {
    pub fn changed(&mut self, key: &KeyChange) {
        if let Some(layout_key) = key.reg_as {

            let mut any_matched = false;

            for sc_key in &mut self.combination {
                let matched = match &sc_key.ident {
                    ScKeyIdent::Coord { x, y } => key.x == *x && key.y == *y,
                    ScKeyIdent::Name(lk) => *layout_key == *lk,
                };

                if matched {
                    any_matched = true;
                }
                
                if key.state && !sc_key.pressed && matched {
                    sc_key.pressed = true;
                }

                if !key.state && sc_key.pressed && matched {
                    if sc_key.required_count == 0 {
                        self.invalid = true;
                    } else {
                        sc_key.pressed = false;
                        sc_key.count += 1;
                    }   
                }
            }

            if !any_matched {
                self.invalid = true;
            }
        }
    }

    pub fn try_apply(&mut self, svc: &mut Services<W, H, C>) -> anyhow::Result<()> {

        if self.invalid {
            self.reset();
            return Ok(());
        }

        let mut matched = true;

        for sc_key in &mut self.combination {
            if sc_key.count < sc_key.required_count {
                matched = false;
            }
        }

        self.action.on_progress(&self.combination, svc)?;

        if matched {
            self.reset();
            self.action.execute(svc)?;
        }

        Ok(())
    }

    pub fn reset(&mut self) {
        for sc_key in &mut self.combination {
            sc_key.pressed = false;
            sc_key.count = 0;
        }

        self.invalid = false;
    }
}

/// Runs every combination against the same key changes.
pub struct SpecialCombinations<const W: usize, const H: usize, const C: usize> {
    combinations: Vec<SpecialCombination<W, H, C>>
}

impl<const W: usize, const H: usize, const C: usize> SpecialCombinations<W, H, C> {
    pub fn new(combinations: Vec<SpecialCombination<W, H, C>>) -> Self {
        Self {
            combinations
        }
    }

    pub fn changed(&mut self, key: &KeyChange) {
        for sc in &mut self.combinations {
            sc.changed(key);
        }
    }

    pub fn try_apply(&mut self, svc: &mut Services<W, H, C>) -> anyhow::Result<()> {
        for sc in &mut self.combinations {
            sc.try_apply(svc)?;
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        for sc in &mut self.combinations {
            sc.reset();
        }
    }
}

#[derive(Debug)]
pub enum ScKeyIdent {
    Coord {
        x: u8,
        y: u8
    },
    Name(LayoutKey)
}

#[derive(Debug)]
pub struct ScKey {
    ident: ScKeyIdent,
    pressed: bool,
    required_count: u8,
    count: u8
}

impl ScKey {
    pub fn new(ident: ScKeyIdent, required_count: Option<u8>) -> Self {
        Self {
            ident,
            pressed: false,
            required_count: required_count.unwrap_or_default(),
            count: 0
        }
    }
}