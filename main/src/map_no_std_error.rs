#[macro_export]
macro_rules! map_err {
    ($expr:expr $(,)?) => { $expr.map_err(|e| anyhow::anyhow!("Error {:?} on: {}:{}", e, file!(), line!()))? };
}