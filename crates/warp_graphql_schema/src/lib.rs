#[cynic::schema("warp-server")]
pub mod schema {}

#[cfg(test)]
#[path = "allowlist_tests.rs"]
mod tests;
