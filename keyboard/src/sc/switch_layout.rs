use crate::services::Services;
use crate::sc::SpecialCombinationAction;

#[derive(Debug)]
pub struct SwitchLayoutSpecialCombinationAction<const W: usize, const H: usize, const C: usize>;

impl<const W: usize, const H: usize, const C: usize> SpecialCombinationAction<W, H, C> for SwitchLayoutSpecialCombinationAction<W, H, C> {
    fn execute(&mut self, svc: &mut Services<W, H, C>)  -> anyhow::Result<()> {
        let new_layout = svc.key_matrix.switch_layout();
        svc.display.show_text(new_layout.name())?;
        Ok(())
    }
}