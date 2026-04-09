use crate::display::Display;
use crate::key_matrix::{KeyChange, KeyMatrix};
use crate::layout::Layouts;

pub mod layout_switch;
pub mod unpair;

pub trait SpecialCombination {
    fn new_scan(&mut self);

    fn changed(&mut self, key: &KeyChange);

    fn apply<const W: usize, const H: usize, const C: usize>(
        &mut self,
        key_matrix: &mut KeyMatrix<W, H>,
        layouts: &'static Layouts<W, H, C>,
        display: &Display,
    ) -> anyhow::Result<()>;

    fn reset(&mut self);
}