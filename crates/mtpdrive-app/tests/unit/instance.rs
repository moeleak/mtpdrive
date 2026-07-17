use super::is_show_signal;

#[test]
fn ui_instance_signal_requires_an_exact_show_message() {
    assert!(is_show_signal(b"show"));
    assert!(!is_show_signal(b""));
    assert!(!is_show_signal(b"show\0"));
    assert!(!is_show_signal(b"show-window"));
    assert!(!is_show_signal(b"noise"));
}
