use std::fmt::Write;

use crate::services::Services;
use crate::sc::SpecialCombinationAction;
use crate::sc::ScKey;

#[derive(Debug)]
pub struct UnpairSpecialCombinationAction<const W: usize, const H: usize, const C: usize> {
    buf: String
}

impl <const W: usize, const H: usize, const C: usize> UnpairSpecialCombinationAction<W, H, C> {
    pub fn new() -> Self {
        Self {
            buf: String::new()
        }
    }
}

impl<const W: usize, const H: usize, const C: usize> SpecialCombinationAction<W, H, C> for UnpairSpecialCombinationAction<W, H, C> {
    fn on_progress(&mut self, combinations: &Vec<ScKey>, svc: &mut Services<W, H, C>)  -> anyhow::Result<()> {
        
        for sc_key in combinations {
            if sc_key.required_count > 0 && sc_key.count > 0 {
                let left = sc_key.required_count - sc_key.count;
                self.buf.clear();
                let _ = write!(self.buf, "Unpair: {}", left);
                svc.display.show_text(&self.buf)?;
            }
        }

        Ok(())
    }

    fn execute(&mut self, svc: &mut Services<W, H, C>)  -> anyhow::Result<()> {
        svc.display.show_text("Unpaired")?;
        svc.keyboard.unpair();
        Ok(())
    }
}