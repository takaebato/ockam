/// The ockam project enroll command arguments contain the enrollment ticket which is sensitive
/// information (because it could be potentially reused), so it should be removed from the user event.
pub fn sanitize_command_arguments(command_args: String) -> String {
    if command_args.starts_with("ockam project enroll") {
        "ockam project enroll".to_string()
    } else {
        command_args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_project_enroll() {
        assert_eq!(
            sanitize_command_arguments("ockam project enroll abcdxyz".to_string()),
            "ockam project enroll".to_string()
        );
        assert_eq!(
            sanitize_command_arguments("ockam node create n1".to_string()),
            "ockam node create n1".to_string()
        );
    }
}
