pub mod protocol;

#[cfg(test)]
mod tests {
    use super::protocol::DaveSession;

    #[test]
    fn test_dave_session_init() {
        let session = DaveSession::new("test_guild".into());
        assert!(!session.is_active());
        assert_eq!(session.epoch(), 0);
    }
}
