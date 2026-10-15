use image::{Rgba, RgbaImage};

use super::BandStitcher;
use crate::stitch::StitchOutcome;

/// 生成没有短周期重复的文档行颜色。
///
/// `row` 为文档绝对行号；返回 RGBA。
fn document_color(row: i32) -> Rgba<u8> {
    let row = row.max(0) as u32;
    Rgba([
        ((row * 7 + 31) % 251) as u8,
        ((row * 11 + 59) % 241) as u8,
        ((row * 13 + 83) % 239) as u8,
        255,
    ])
}

/// 生成带固定侧栏和可选顶栏的滚动页面。
///
/// `offset` 为主内容位移，`sidebar_width` 为侧栏宽度，`right` 表示侧栏在右侧，`header_height` 为顶栏高度；返回一帧。
fn sidebar_frame(offset: i32, sidebar_width: u32, right: bool, header_height: u32) -> RgbaImage {
    let mut image = RgbaImage::from_pixel(800, 600, Rgba([0, 0, 0, 255]));
    let content_left = if right { 0 } else { sidebar_width };
    let sidebar_left = if right { 800 - sidebar_width } else { 0 };
    for y in 0..600 {
        let content = document_color(offset + y as i32 - header_height as i32);
        let sidebar = document_color(y as i32 * 3 + 701);
        for x in content_left..content_left + (800 - sidebar_width) {
            image.put_pixel(x, y, content);
        }
        for x in sidebar_left..sidebar_left + sidebar_width {
            image.put_pixel(x, y, sidebar);
        }
    }
    if header_height > 0 {
        let header = Rgba([210, 220, 230, 255]);
        for y in 0..header_height {
            for x in 0..800 {
                image.put_pixel(x, y, header);
            }
        }
    }
    image
}

/// 生成固定目录侧栏随章节切换高亮的网页帧。
///
/// `offset` 为正文位移，`section` 为高亮目录序号；返回 1280x800 帧。
fn highlighted_sidebar_frame(offset: i32, section: i32) -> RgbaImage {
    let mut image = RgbaImage::from_pixel(1280, 800, Rgba([0, 0, 0, 255]));
    for y in 0..800 {
        let content = document_color(offset + y as i32);
        let side = if (y / 24) % 2 == 1 {
            Rgba([245, 246, 248, 255])
        } else {
            Rgba([232, 235, 240, 255])
        };
        for x in 360..1280 {
            image.put_pixel(x, y, content);
        }
        for x in 0..360 {
            image.put_pixel(x, y, side);
        }
    }
    let highlight = Rgba([70, 120, 220, 255]);
    let top = (40 + section * 48).clamp(0, 760) as u32;
    for y in top..top + 40 {
        for x in 16..344 {
            image.put_pixel(x, y, highlight);
        }
    }
    image
}

/// 生成只有中间正文滚动、左侧导航固定的稀疏文档。
///
/// `offset` 为文档位移，`height` 为图像高度；返回一帧。
fn sparse_document_frame(offset: i32, height: u32) -> RgbaImage {
    let mut image = RgbaImage::from_pixel(800, height, Rgba([255, 255, 255, 255]));
    for y in 0..height {
        let sidebar = document_color(y as i32 * 3 + 701);
        for x in 0..224 {
            image.put_pixel(x, y, sidebar);
        }
        let row = offset + y as i32;
        if row % 28 < 6 {
            let width = 120 + ((row / 28) * 37).rem_euclid(180);
            let ink = Rgba([35, 40, 45, 255]);
            for x in 250..(250 + width as u32).min(800) {
                image.put_pixel(x, y, ink);
            }
        }
    }
    image
}

/// 生成具有相似空白页边、正文含轻微采集噪声的文档帧。
///
/// `offset` 为文档位移，`noisy` 表示正文增加一个亮度级别；返回当前可见帧。
fn blank_margin_frame(offset: i32, noisy: bool) -> RgbaImage {
    let mut image = RgbaImage::from_pixel(160, 600, Rgba([255, 255, 255, 255]));
    for y in 0..600 {
        let row = offset + y as i32;
        if (150..650).contains(&row) {
            let mut color = document_color(row);
            if noisy {
                color.0[0] = color.0[0].saturating_add(1);
                color.0[1] = color.0[1].saturating_add(1);
                color.0[2] = color.0[2].saturating_add(1);
            }
            for x in 0..160 {
                image.put_pixel(x, y, color);
            }
        }
    }
    image
}

#[test]
/// 覆盖不同侧栏位置、占比和浏览器顶栏；无参数、无返回值。
fn appends_whole_content_with_static_sidebar() {
    let cases = [(224, false, 0), (224, true, 0), (320, false, 0), (224, false, 60)];
    for (sidebar_width, right, header_height) in cases {
        let mut stitcher = BandStitcher::new();
        assert!(matches!(
            stitcher.push(sidebar_frame(0, sidebar_width, right, header_height)),
            StitchOutcome::FirstFrame
        ));
        for offset in [180, 360, 540, 720] {
            match stitcher.push(sidebar_frame(offset, sidebar_width, right, header_height)) {
                StitchOutcome::Appended { added } => assert_eq!(added, 180),
                other => panic!("expected append at {offset}, got {other:?}"),
            }
        }
        let full = stitcher.full_image().expect("full image");
        assert_eq!(full.dimensions(), (800, 1320));
        let content_x = if right { 100 } else { 700 };
        for y in header_height..full.height() {
            assert_eq!(
                *full.get_pixel(content_x, y),
                document_color(y as i32 - header_height as i32),
                "content mismatch at y={y}"
            );
        }
        assert!(matches!(
            stitcher.push(sidebar_frame(720, sidebar_width, right, header_height)),
            StitchOutcome::NoProgress
        ));
    }
}

#[test]
/// 倒退或返回已知内容时不重复追加；无参数、无返回值。
fn revisits_known_content() {
    let mut stitcher = BandStitcher::new();
    for offset in [0, 180, 360] {
        stitcher.push(sidebar_frame(offset, 224, false, 60));
    }
    let before = stitcher.full_image().expect("before").as_ref().clone();
    for offset in [180, 0, 180, 360] {
        assert!(
            matches!(
                stitcher.push(sidebar_frame(offset, 224, false, 60)),
                StitchOutcome::NoProgress
            ),
            "revisit {offset}"
        );
        assert_eq!(stitcher.full_image().expect("stable").as_ref(), &before);
    }
    match stitcher.push(sidebar_frame(540, 224, false, 60)) {
        StitchOutcome::Appended { added } => assert_eq!(added, 180),
        other => panic!("expected append after revisit, got {other:?}"),
    }
}

#[test]
/// 反向滚动时保留完整正文，且只在顶部保留一份固定顶栏；无参数、无返回值。
fn prepends_past_fixed_navigation() {
    let mut stitcher = BandStitcher::new();
    stitcher.push(sidebar_frame(360, 224, false, 60));
    for offset in [180, 0] {
        match stitcher.push(sidebar_frame(offset, 224, false, 60)) {
            StitchOutcome::Appended { added } => assert_eq!(added, 180),
            other => panic!("expected prepend, got {other:?}"),
        }
    }
    let full = stitcher.full_image().expect("full");
    assert_eq!(full.height(), 960);
    for y in 60..full.height() {
        assert_eq!(*full.get_pixel(700, y), document_color(y as i32 - 60));
    }
}

#[test]
/// 侧栏目录高亮变化、单次滚动超过预测窗口时仍按真实位移拼接；无参数、无返回值。
fn follows_large_steps_with_sidebar_highlight() {
    let mut stitcher = BandStitcher::new();
    let mut offset = 0;
    stitcher.push(highlighted_sidebar_frame(offset, 0));
    for (index, step) in [240, 410, 180, 460, 300, 520].into_iter().enumerate() {
        offset += step;
        match stitcher.push(highlighted_sidebar_frame(offset, index as i32 + 1)) {
            StitchOutcome::Appended { added } => assert_eq!(added, step as u32),
            other => panic!("expected append {step}, got {other:?}"),
        }
    }
    let full = stitcher.full_image().expect("full");
    assert_eq!(full.height(), offset as u32 + 800);
    let mut y = 0;
    while y < full.height() {
        assert_eq!(*full.get_pixel(900, y), document_color(y as i32));
        y += 7;
    }
}

#[test]
/// 稀疏正文不会被静止侧栏误判为未滚动；无参数、无返回值。
fn stitches_sparse_documents() {
    let mut stitcher = BandStitcher::new();
    stitcher.push(sparse_document_frame(0, 600));
    for offset in [137, 274, 411] {
        match stitcher.push(sparse_document_frame(offset, 600)) {
            StitchOutcome::Appended { added } => assert_eq!(added, 137),
            other => panic!("expected sparse append, got {other:?}"),
        }
    }
    let full = stitcher.full_image().expect("full");
    assert_eq!(full.height(), 1011);
    let reference = sparse_document_frame(0, 1011);
    for y in 0..full.height() {
        for x in 224..800 {
            assert_eq!(full.get_pixel(x, y), reference.get_pixel(x, y));
        }
    }
}

#[test]
/// 排除固定侧栏后仍拒绝没有正文重叠的页面；无参数、无返回值。
fn rejects_unrelated_content() {
    let mut stitcher = BandStitcher::new();
    let first = sidebar_frame(0, 224, false, 0);
    stitcher.push(first.clone());
    let mut unrelated = first.clone();
    for y in 0..600 {
        for x in 224..800 {
            unrelated.put_pixel(x, y, Rgba([0, 0, 0, 255]));
        }
    }
    assert!(matches!(stitcher.push(unrelated), StitchOutcome::NoMatch));
    assert_eq!(stitcher.full_image().expect("unchanged").as_ref(), &first);
}

#[test]
/// 相同空白页边不能击败具有轻微采集误差的真实正文重叠；无参数、无返回值。
fn prefers_text_overlap_to_blank_margins() {
    let mut stitcher = BandStitcher::new();
    stitcher.push(blank_margin_frame(0, false));
    match stitcher.push(blank_margin_frame(200, true)) {
        StitchOutcome::Appended { added } => assert_eq!(added, 200),
        other => panic!("expected noisy append, got {other:?}"),
    }
    assert_eq!(stitcher.full_image().expect("full").height(), 800);
}

#[test]
/// 完全空白的静止帧只报告没有进展；无参数、无返回值。
fn keeps_blank_frames_idle() {
    let mut stitcher = BandStitcher::new();
    let frame = RgbaImage::from_pixel(160, 600, Rgba([255, 255, 255, 255]));
    stitcher.push(frame.clone());
    assert!(matches!(stitcher.push(frame), StitchOutcome::NoProgress));
    assert_eq!(stitcher.stats().frame_count, 1);
}
