//! Pure composition of top-document and iframe fields into framebuffer pixels.
#![allow(dead_code)]

use super::agent::Fields;
use super::{geometry, protocol};

pub(super) struct Child<'a> {
    /// Iframe content rect in the top document's visual-viewport CSS coordinates.
    pub offset: [f64; 4],
    pub boxes: &'a protocol::Boxes,
}

pub(super) fn compose(
    top: &protocol::Boxes,
    children: &[Child<'_>],
    map: &geometry::ContentMap,
) -> Fields {
    let top_boxes = top
        .boxes
        .iter()
        .copied()
        .enumerate()
        .map(|(i, rect)| (Some(rect), top.focused == Some(i)));
    let child_boxes = children.iter().flat_map(|child| {
        child
            .boxes
            .boxes
            .iter()
            .copied()
            .enumerate()
            .map(|(i, rect)| {
                (
                    child_box(&top.vp, child, rect),
                    child.boxes.focused == Some(i),
                )
            })
    });
    let visible = top_boxes
        .chain(child_boxes)
        .filter_map(|(rect, focused)| Some((geometry::to_screen(map, rect?)?, focused)))
        .take(128);
    let mut boxes = Vec::new();
    let mut focused = None;
    for (rect, is_focused) in visible {
        if is_focused && focused.is_none() {
            focused = Some(boxes.len() as u8);
        }
        boxes.push(rect);
    }
    Fields::new(map.screen.0, map.screen.1, focused, boxes)
        .expect("mapped fields are bounded and clipped to the screen")
}

fn child_box(top: &protocol::Viewport, child: &Child<'_>, rect: [f64; 4]) -> Option<[f64; 4]> {
    let vp = &child.boxes.vp;
    if !vp
        .inner
        .into_iter()
        .chain([vp.vv_scale])
        .all(|v| v.is_finite() && v > 0.)
    {
        return None;
    }
    let [x, y, w, h] = geometry::clip(rect, vp.vv)?;
    let [left, top_edge, width, height] = child.offset;
    let sx = width / vp.inner[0] * vp.vv_scale;
    let sy = height / vp.inner[1] * vp.vv_scale;
    let local = [
        left + (x - vp.vv[0]) * sx,
        top_edge + (y - vp.vv[1]) * sy,
        w * sx,
        h * sy,
    ];
    let [x, y, w, h] = geometry::clip(local, child.offset)?;
    // frameRect already removed the top visual-viewport offsets; the map removes them
    // for raw layout boxes, so put them back once for child boxes.
    Some([x + top.vv[0], y + top.vv[1], w, h])
}

#[cfg(test)]
mod tests {
    use super::super::agent::FieldBox;
    use super::*;

    fn report(boxes: Vec<[f64; 4]>, focused: Option<usize>) -> protocol::Boxes {
        protocol::Boxes {
            doc: 1,
            vp: protocol::Viewport {
                vv: [0., 0., 390., 844.],
                vv_scale: 1.,
                inner: [390., 844.],
                dpr: 2.,
                screen: [390., 844.],
                zoom: 1.,
            },
            boxes,
            focused,
        }
    }

    fn map(top: &protocol::Boxes) -> geometry::ContentMap {
        geometry::content_map(
            &top.vp,
            &geometry::Window {
                left: 0.,
                top: 0.,
                width: 390.,
                height: 844.,
                fullscreen: true,
            },
        )
        .unwrap()
    }

    #[test]
    fn tests_that_top_boxes_keep_focus_after_invisible_fields_are_removed() {
        let top = report(vec![[-100., 0., 10., 10.], [20., 57., 300., 40.]], Some(1));
        let fields = compose(&top, &[], &map(&top));
        assert_eq!((fields.width(), fields.height()), (390, 844));
        assert_eq!(
            fields.boxes(),
            &[FieldBox {
                x: 20,
                y: 57,
                w: 300,
                h: 40
            }]
        );
        assert_eq!(fields.focused(), Some(0));
        let hidden = report(top.boxes, Some(0));
        assert_eq!(compose(&hidden, &[], &map(&hidden)).focused(), None);
    }

    #[test]
    fn tests_that_child_boxes_use_iframe_offsets_and_clip_to_its_content() {
        let top = report(vec![[1., 2., 3., 4.]], None);
        let mut child = report(
            vec![
                [-5., -10., 20., 30.],
                [90., 40., 30., 30.],
                [100., 0., 10., 10.],
            ],
            Some(1),
        );
        child.vp.inner = [100., 50.];
        child.vp.vv = [0., 0., 100., 50.];
        let fields = compose(
            &top,
            &[Child {
                offset: [30., 40., 100., 50.],
                boxes: &child,
            }],
            &map(&top),
        );
        assert_eq!(
            fields.boxes(),
            &[
                FieldBox {
                    x: 1,
                    y: 2,
                    w: 3,
                    h: 4
                },
                FieldBox {
                    x: 30,
                    y: 40,
                    w: 15,
                    h: 20
                },
                FieldBox {
                    x: 120,
                    y: 80,
                    w: 10,
                    h: 10
                },
            ]
        );
        assert_eq!(fields.focused(), Some(2));
    }

    #[test]
    fn tests_that_child_visual_viewport_and_rendered_iframe_size_are_composed_once() {
        let mut top = report(vec![], None);
        top.vp.vv = [10., 20., 195., 422.];
        top.vp.vv_scale = 2.;
        let mut child = report(vec![[15., 27., 10., 5.]], Some(0));
        child.vp.inner = [100., 50.];
        child.vp.vv = [5., 7., 50., 25.];
        child.vp.vv_scale = 2.;
        let fields = compose(
            &top,
            &[Child {
                offset: [30., 40., 200., 100.],
                boxes: &child,
            }],
            &map(&top),
        );
        assert_eq!(
            fields.boxes(),
            &[FieldBox {
                x: 140,
                y: 240,
                w: 80,
                h: 40
            }]
        );
        assert_eq!(fields.focused(), Some(0));
    }

    #[test]
    fn tests_that_composition_caps_all_frames_at_128_and_never_emits_invalid_focus() {
        let top = report(vec![[0., 0., 1., 1.]; 127], None);
        let child = report(vec![[2., 2., 1., 1.]; 3], Some(1));
        let fields = compose(
            &top,
            &[Child {
                offset: [0., 0., 390., 844.],
                boxes: &child,
            }],
            &map(&top),
        );
        assert_eq!(fields.boxes().len(), 128);
        assert_eq!(fields.focused(), None);
        let invalid = report(vec![[f64::NAN, 0., 1., 1.], [0., 0., -1., 1.]], Some(99));
        let fields = compose(&invalid, &[], &map(&invalid));
        assert!(fields.boxes().is_empty());
        assert_eq!(fields.focused(), None);
    }

    #[test]
    fn tests_that_invalid_child_geometry_is_ignored_without_losing_top_fields() {
        let top = report(vec![[1., 2., 3., 4.]], Some(0));
        let child = report(vec![[1., 2., 3., 4.]], Some(0));
        for offset in [
            [f64::NAN, 0., 10., 10.],
            [0., 0., 0., 10.],
            [0., 0., 10., -1.],
            [0., 0., f64::INFINITY, 10.],
        ] {
            let fields = compose(
                &top,
                &[Child {
                    offset,
                    boxes: &child,
                }],
                &map(&top),
            );
            assert_eq!(fields.boxes().len(), 1);
            assert_eq!(fields.focused(), Some(0));
        }
        let mut child = child;
        child.vp.inner[0] = 0.;
        let fields = compose(
            &top,
            &[Child {
                offset: [0., 0., 10., 10.],
                boxes: &child,
            }],
            &map(&top),
        );
        assert_eq!(fields.boxes().len(), 1);
    }
}
