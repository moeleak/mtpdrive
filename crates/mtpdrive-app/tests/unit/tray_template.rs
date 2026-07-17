use super::{SIZE, rgba};

fn alpha_at(rgba: &[u8], x: u32, y: u32) -> u8 {
    rgba[((y * SIZE + x) * 4 + 3) as usize]
}

#[test]
fn artwork_has_clear_details_and_padding() {
    let rgba = rgba();
    assert_eq!(rgba.len(), (SIZE * SIZE * 4) as usize);
    assert_eq!(alpha_at(&rgba, 0, 0), 0);
    assert!(alpha_at(&rgba, 18, 7) > 240);
    assert!(alpha_at(&rgba, 14, 12) < 16);
    assert!(alpha_at(&rgba, 18, 17) > 240);
    assert_eq!(alpha_at(&rgba, 8, 21), 0);
    assert!(alpha_at(&rgba, 18, 25) < 16);
    assert!(alpha_at(&rgba, 18, 32) > 240);
    assert_eq!(alpha_at(&rgba, 35, 35), 0);
}
