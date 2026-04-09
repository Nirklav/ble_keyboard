use crate::display::Display;
use crate::key_matrix::{KeyChange, KeyMatrix};
use crate::layout::Layouts;
use crate::sc::SpecialCombination;

pub struct Unpair {

}

impl SpecialCombination for Unpair {
    fn new_scan(&mut self) {
        todo!()
    }

    fn changed(&mut self, key: &KeyChange) {
        todo!()
    }

    fn apply<const W: usize, const H: usize, const C: usize>(
        &mut self,
        _key_matrix: &mut KeyMatrix<W, H>,
        _layouts: &'static Layouts<W, H, C>,
        _display: &Display,
    ) -> anyhow::Result<()> {
        todo!()
    }

    fn reset(&mut self) {
        todo!()
    }
}
