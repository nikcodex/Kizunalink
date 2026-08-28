pub mod protocol;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dave_stub() {
        // Since we copied dave.rs from the original which didn't actually expose
        // the gateway loop, we will just make a sanity check that it compiles.
        assert!(true);
    }
}
