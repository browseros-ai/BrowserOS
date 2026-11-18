//! Pure CSS-to-framebuffer mapping, including the visual viewport and browser chrome.
#![allow(dead_code)]

use super::agent::FieldBox;
use super::protocol;

pub(super) struct Window {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
    pub fullscreen: bool,
}
pub(super) struct ContentMap {
    origin: (f64, f64),
    px_per_css: f64,
    pub(super) screen: (u16, u16),
    viewport: [f64; 4],
}

pub(super) fn content_map(vp: &protocol::Viewport, win: &Window) -> Option<ContentMap> {
    if ![win.left, win.top].into_iter().all(f64::is_finite)
        || ![win.width, win.height, vp.vv_scale, vp.dpr, vp.zoom]
            .into_iter()
            .chain(vp.inner)
            .chain(vp.screen)
            .all(positive)
        || !valid_rect(vp.vv)
    {
        return None;
    }
    let (origin, px_per_css, width, height) = if win.fullscreen {
        // The visible CSS width spans the phone preview and already includes page scale.
        ((0., 0.), win.width / vp.vv[2], win.width, win.height)
    } else {
        // Invert desktop::viewport_point: CSS = (pixel / scale - origin) / zoom.
        let scale = vp.dpr / vp.zoom;
        let inner_width = vp.inner[0] * vp.zoom;
        let inner_height = vp.inner[1] * vp.zoom;
        if ![inner_width, inner_height].into_iter().all(f64::is_finite) {
            return None;
        }
        let border = ((win.width - inner_width) / 2.).max(0.);
        let toolbar = (win.height - inner_height - border).max(0.);
        (
            ((win.left + border) * scale, (win.top + toolbar) * scale),
            vp.dpr * vp.vv_scale,
            vp.screen[0] * scale,
            vp.screen[1] * scale,
        )
    };
    let origin = (
        origin.0 - vp.vv[0] * px_per_css,
        origin.1 - vp.vv[1] * px_per_css,
    );
    if !positive(px_per_css) || ![origin.0, origin.1].into_iter().all(f64::is_finite) {
        return None;
    }
    Some(ContentMap {
        origin,
        px_per_css,
        screen: (dimension(width)?, dimension(height)?),
        viewport: vp.vv,
    })
}

/// Raw getBoundingClientRect CSS coordinates → visible, outward-rounded screen pixels.
pub(super) fn to_screen(map: &ContentMap, css: [f64; 4]) -> Option<FieldBox> {
    let [x, y, w, h] = clip(css, map.viewport)?;
    let left = map.origin.0 + x * map.px_per_css;
    let top = map.origin.1 + y * map.px_per_css;
    let right = map.origin.0 + (x + w) * map.px_per_css;
    let bottom = map.origin.1 + (y + h) * map.px_per_css;
    if ![left, top, right, bottom].into_iter().all(f64::is_finite) {
        return None;
    }
    let left = left.max(0.).floor();
    let top = top.max(0.).floor();
    let right = right.min(f64::from(map.screen.0)).ceil();
    let bottom = bottom.min(f64::from(map.screen.1)).ceil();
    if right <= left || bottom <= top {
        return None;
    }
    Some(FieldBox {
        x: left as u16,
        y: top as u16,
        w: (right - left) as u16,
        h: (bottom - top) as u16,
    })
}

pub(super) fn clip(rect: [f64; 4], bounds: [f64; 4]) -> Option<[f64; 4]> {
    if !valid_rect(rect) || !valid_rect(bounds) {
        return None;
    }
    let x = rect[0].max(bounds[0]);
    let y = rect[1].max(bounds[1]);
    let right = (rect[0] + rect[2]).min(bounds[0] + bounds[2]);
    let bottom = (rect[1] + rect[3]).min(bounds[1] + bounds[3]);
    (right > x && bottom > y).then_some([x, y, right - x, bottom - y])
}

fn valid_rect([x, y, w, h]: [f64; 4]) -> bool {
    [x, y, x + w, y + h].into_iter().all(f64::is_finite) && positive(w) && positive(h)
}

fn positive(value: f64) -> bool {
    value.is_finite() && value > 0.
}

fn dimension(value: f64) -> Option<u16> {
    let rounded = value.round();
    (value.is_finite() && rounded >= 1. && rounded <= f64::from(u16::MAX)).then_some(rounded as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone() -> (protocol::Viewport, Window) {
        (
            protocol::Viewport {
                vv: [0., 0., 390., 844.],
                vv_scale: 1.,
                inner: [390., 844.],
                dpr: 2.,
                screen: [390., 844.],
                zoom: 1.,
            },
            Window {
                left: 0.,
                top: 0.,
                width: 390.,
                height: 844.,
                fullscreen: true,
            },
        )
    }

    #[test]
    fn tests_that_phone_boxes_match_measured_preview_pixels_without_dpr_scaling() {
        let (mut vp, mut win) = phone();
        for (scale, expected) in [
            (
                1.,
                FieldBox {
                    x: 20,
                    y: 57,
                    w: 300,
                    h: 40,
                },
            ),
            (
                2.,
                FieldBox {
                    x: 40,
                    y: 114,
                    w: 600,
                    h: 80,
                },
            ),
        ] {
            win.width = 390. * scale;
            win.height = 844. * scale;
            vp.dpr = 3.;
            let map = content_map(&vp, &win).unwrap();
            assert_eq!(map.screen, (win.width as u16, win.height as u16));
            assert_eq!(to_screen(&map, [20., 57., 300., 40.]), Some(expected));
        }
    }

    #[test]
    fn tests_that_scaled_phone_fields_match_the_framebuffer_for_both_inner_widths() {
        let (mut vp, mut win) = phone();
        win.width = 780.;
        win.height = 1592.;
        vp.screen = [390., 796.];
        vp.vv = [0., 0., 490., 1000.];
        vp.vv_scale = 0.796;
        for inner in [[980., 2001.], [490., 1001.]] {
            vp.inner = inner;
            let map = content_map(&vp, &win).unwrap();
            for (css, expected) in [
                // The recorded custname rect, rounded outwards in framebuffer pixels.
                ([117.3125, 16., 185., 21.], [186, 25, 296, 34]),
                ([118., 16., 184., 20.], [187, 25, 294, 33]),
            ] {
                let field = to_screen(&map, css).unwrap();
                assert_eq!([field.x, field.y, field.w, field.h], expected, "{inner:?}");
            }
        }
    }

    #[test]
    fn tests_that_desktop_toolbar_is_derived_from_window_and_inner_size() {
        let (mut vp, mut win) = phone();
        vp.inner = [780., 1601.];
        vp.vv = [0., 0., 780., 1601.];
        vp.screen = [780., 1688.];
        vp.dpr = 1.;
        win.width = 780.;
        win.height = 1688.;
        win.fullscreen = false;
        let map = content_map(&vp, &win).unwrap();
        assert_eq!(map.screen, (780, 1688));
        assert_eq!(
            to_screen(&map, [20., 57., 300., 40.]),
            Some(FieldBox {
                x: 20,
                y: 144,
                w: 300,
                h: 40
            })
        );
        win.height += 13.;
        let map = content_map(&vp, &win).unwrap();
        assert_eq!(to_screen(&map, [20., 57., 300., 40.]).unwrap().y, 157);
    }

    #[test]
    fn tests_that_desktop_mapping_inverts_window_borders_page_zoom_and_dpi() {
        let (mut vp, _) = phone();
        for (outer, inner, zoom, dpr, origin, expected) in [
            (
                [1002., 702.],
                [1000., 600.],
                1.,
                1.,
                [100., 50.],
                [201, 251],
            ),
            ([1920., 1080.], [960., 496.5], 2., 2., [0., 0.], [400, 287]),
            ([960., 540.], [960., 453.], 1., 2., [0., 0.], [400, 374]),
        ] {
            vp.inner = inner;
            vp.vv = [0., 0., inner[0], inner[1]];
            vp.screen = [1920., 1080.];
            vp.zoom = zoom;
            vp.dpr = dpr;
            let win = Window {
                left: origin[0],
                top: origin[1],
                width: outer[0],
                height: outer[1],
                fullscreen: false,
            };
            let map = content_map(&vp, &win).unwrap();
            let point = if zoom == 1. && dpr == 1. { 100. } else { 200. };
            let b = to_screen(&map, [point, 100., 10., 10.]).unwrap();
            assert_eq!([b.x, b.y], expected);
            assert_eq!(b.w, (10. * dpr) as u16);
            assert_eq!(
                map.screen,
                ((1920. * dpr / zoom) as u16, (1080. * dpr / zoom) as u16)
            );
        }
    }

    #[test]
    fn tests_that_visual_viewport_offset_and_scale_precede_preview_scaling() {
        let (mut vp, mut win) = phone();
        vp.inner = [980., 2001.];
        vp.vv = [10., 20., 195., 422.];
        vp.vv_scale = 2.;
        win.width *= 2.;
        win.height *= 2.;
        let map = content_map(&vp, &win).unwrap();
        assert_eq!(
            to_screen(&map, [20., 57., 30., 40.]),
            Some(FieldBox {
                x: 40,
                y: 148,
                w: 120,
                h: 160
            })
        );
        assert_eq!(
            to_screen(&map, [0., 0., 20., 30.]),
            Some(FieldBox {
                x: 0,
                y: 0,
                w: 40,
                h: 40
            })
        );
        assert!(to_screen(&map, [205., 20., 10., 10.]).is_none());

        // The keyboard clips the bottom of the field at the same pixel scale.
        vp.vv[3] = 60.;
        let map = content_map(&vp, &win).unwrap();
        let field = to_screen(&map, [20., 57., 30., 40.]).unwrap();
        assert_eq!([field.x, field.y, field.w, field.h], [40, 148, 120, 92]);
    }

    #[test]
    fn tests_that_windowed_desktop_browser_zoom_and_pinch_each_apply_once() {
        let (mut vp, mut win) = phone();
        vp.screen = [1200., 800.];
        vp.dpr = 2.;
        vp.zoom = 2.;
        vp.vv_scale = 2.;
        win.width = 1200.;
        win.height = 800.;
        win.fullscreen = false;
        vp.inner = [600., 350.];
        // The visual viewport excludes the scrollbar and starts below the page top.
        vp.vv = [0., 20., 295., 175.];
        let map = content_map(&vp, &win).unwrap();
        let field = to_screen(&map, [10., 30., 10., 5.]).unwrap();
        assert_eq!([field.x, field.y, field.w, field.h], [40, 140, 40, 20]);
    }

    #[test]
    fn tests_that_boxes_clip_to_viewport_and_screen_with_outward_pixel_rounding() {
        let (vp, win) = phone();
        let map = content_map(&vp, &win).unwrap();
        assert_eq!(
            to_screen(&map, [-5., -3., 20., 10.]),
            Some(FieldBox {
                x: 0,
                y: 0,
                w: 15,
                h: 7
            })
        );
        assert_eq!(
            to_screen(&map, [389.2, 843.2, 10., 10.]),
            Some(FieldBox {
                x: 389,
                y: 843,
                w: 1,
                h: 1
            })
        );
        assert_eq!(
            to_screen(&map, [1.2, 2.3, 3.1, 4.2]),
            Some(FieldBox {
                x: 1,
                y: 2,
                w: 4,
                h: 5
            })
        );
        for b in [
            [390., 0., 1., 1.],
            [0., -3., 1., 2.],
            [0., 0., 0., 1.],
            [0., 0., -1., 1.],
            [f64::NAN, 0., 1., 1.],
            [0., 0., f64::INFINITY, 1.],
        ] {
            assert!(to_screen(&map, b).is_none(), "{b:?}");
        }
    }

    #[test]
    fn tests_that_desktop_overflow_is_rejected_before_deriving_chrome() {
        let (mut vp, mut win) = phone();
        win.fullscreen = false;
        vp.zoom = f64::MAX;
        vp.dpr = f64::MAX;
        assert!(content_map(&vp, &win).is_none());
    }

    #[test]
    fn tests_that_invalid_or_unrepresentable_maps_are_rejected() {
        for bad in [0., -1., f64::NAN, f64::INFINITY, 65536.] {
            let (vp, mut win) = phone();
            win.width = bad;
            assert!(content_map(&vp, &win).is_none(), "width {bad}");
        }
        for bad in [0., -1., f64::NAN, f64::INFINITY] {
            let (mut vp, win) = phone();
            vp.vv_scale = bad;
            assert!(content_map(&vp, &win).is_none());
            let (mut vp, win) = phone();
            vp.inner[0] = bad;
            assert!(content_map(&vp, &win).is_none());
        }
        let (mut vp, win) = phone();
        vp.vv[0] = f64::INFINITY;
        assert!(content_map(&vp, &win).is_none());
        let (mut vp, mut win) = phone();
        win.fullscreen = false;
        vp.screen[0] = 65536.;
        vp.dpr = 1.;
        assert!(content_map(&vp, &win).is_none());
    }
}
