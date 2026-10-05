//! The similar images report on real image files, through the menu.

use crossterm::event::KeyCode;

use super::{browser, index_of, menu, open, press, rows, tick_until};
use crate::reports::{MenuItem, ReportKind};

/// A picture with broad shapes, like a photo (not noise, which no two
/// sizes of would share).
fn photo(w: u32, h: u32) -> image::RgbImage {
    image::RgbImage::from_fn(w, h, |x, y| {
        let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
        let r = (255.0 * fx) as u8;
        let g = (255.0 * (1.0 - fy)) as u8;
        let b = if (fx - 0.5).powi(2) + (fy - 0.4).powi(2) < 0.04 {
            230
        } else {
            40
        };
        image::Rgb([r, g, b])
    })
}

#[test]
fn resized_copies_are_grouped_and_the_largest_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let pics = dir.path().join("Pictures");
    std::fs::create_dir_all(&pics).unwrap();
    // Uncompressed BMP, so each file is over the 100 KiB minimum.
    photo(320, 240).save(pics.join("holiday.bmp")).unwrap();
    image::DynamicImage::ImageRgb8(photo(320, 240))
        .resize(240, 180, image::imageops::FilterType::Triangle)
        .to_rgb8()
        .save(pics.join("holiday-small.bmp"))
        .unwrap();
    let other = image::RgbImage::from_fn(320, 240, |x, y| {
        let v = if (x / 40 + y / 40) % 2 == 0 { 250 } else { 10 };
        image::Rgb([v, 255 - v, v / 2])
    });
    other.save(pics.join("chess.bmp")).unwrap();

    let mut app = open(dir.path());
    menu(
        &mut app,
        index_of(MenuItem::Report(ReportKind::SimilarImages)),
    );
    tick_until(&mut app, "the similar images", |a| {
        browser(a).similar_job.is_none() && browser(a).results.is_some()
    });
    let list = browser(&app).results.as_ref().unwrap();
    assert_eq!(list.rows.len(), 1, "{:?}", rows(&app));
    assert_eq!(list.rows[0].label, "holiday.bmp");
    assert_eq!(list.rows[0].detail, "2 benzer görsel · en büyüğü 320×240");

    // Space puts all but the largest image into the basket.
    press(&mut app, KeyCode::Char(' '));
    let b = browser(&app);
    let names: Vec<&str> = b.basket.items().iter().map(|&id| b.tree.name(id)).collect();
    assert_eq!(names, ["holiday-small.bmp"]);

    // The group's images show their size; the largest is marked. (Space
    // moved the cursor on; back to the group.)
    press(&mut app, KeyCode::Home);
    press(&mut app, KeyCode::Enter);
    let list = browser(&app).results.as_ref().unwrap();
    let details: Vec<&str> = list.rows.iter().map(|r| r.detail.as_str()).collect();
    assert_eq!(details, ["320×240 · en büyük", "240×180"]);
}

#[test]
fn without_images_the_report_says_why() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), vec![b'x'; 200_000]).unwrap();
    let mut app = open(dir.path());
    menu(
        &mut app,
        index_of(MenuItem::Report(ReportKind::SimilarImages)),
    );
    tick_until(&mut app, "the report", |a| browser(a).results.is_some());
    let list = browser(&app).results.as_ref().unwrap();
    assert!(list.rows.is_empty());
    assert!(list.note.contains("HEIC"), "{}", list.note);
    assert!(browser(&app).similar_job.is_none(), "nothing to search");
}
