#[macro_export]
macro_rules! map_err {
    ($expr:expr $(,)?) => { $expr.map_err(|_| anyhow::anyhow!("Error on: {}:{}", file!(), line!()))? };
}