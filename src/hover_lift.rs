use gpui_kit::base::motion::{Transition, transition};
use gpui_kit::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId,
    InspectorElementId, InteractiveElement as _, IntoElement, LayoutId,
    ParentElement, Pixels, RenderOnce, StatefulInteractiveElement as _, Styled,
    Window, div, point, px,
};

#[derive(IntoElement)]
pub struct HoverLift {
    child: AnyElement,
    distance: Pixels,
    timing: Transition,
    disabled: bool,
}

impl HoverLift {
    pub fn new(
        child: impl IntoElement,
        distance: Pixels,
        timing: Transition,
        disabled: bool,
    ) -> Self {
        Self {
            child: child.into_any_element(),
            distance,
            timing,
            disabled,
        }
    }
}

impl RenderOnce for HoverLift {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let hovered = window.use_state(cx, |_, _| false);
        let target =
            if *hovered.read(cx) && !self.disabled && !cx.reduce_motion() {
                -self.distance
            } else {
                px(0.)
            };
        let offset = if self.disabled || cx.reduce_motion() {
            px(0.)
        } else {
            transition("hover-lift-offset", target, self.timing, window, cx)
        };
        let pointer_hover = hovered.clone();
        div()
            .id("hover-lift")
            .max_w_full()
            .on_hover(move |is_hovered, _, cx| {
                if !is_hovered {
                    hovered.update(cx, |value, cx| {
                        if *value {
                            *value = false;
                            cx.notify();
                        }
                    });
                }
            })
            .on_mouse_move(move |event, _, cx| {
                if event.pressed_button.is_none() {
                    pointer_hover.update(cx, |value, cx| {
                        if !*value {
                            *value = true;
                            cx.notify();
                        }
                    });
                }
            })
            .child(Translated {
                child: self.child,
                offset,
            })
    }
}

// Prepaint translation preserves wrapping and the stationary hover hitbox.
struct Translated {
    child: AnyElement,
    offset: Pixels,
}

impl IntoElement for Translated {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Translated {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(
        &self,
    ) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_element_offset(point(px(0.), self.offset), |window| {
            self.child.prepaint(window, cx);
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{
        Context, Modifiers, Render, TestAppContext, VisualTestContext,
    };
    use std::time::Duration;

    struct Fixture {
        disabled: bool,
    }

    impl Render for Fixture {
        fn render(
            &mut self,
            _: &mut Window,
            _: &mut Context<Self>,
        ) -> impl IntoElement {
            div().size_full().p(px(20.)).child(HoverLift::new(
                div().w(px(80.)).h(px(32.)).debug_selector(|| "face".into()),
                px(3.),
                Transition::new(Duration::from_millis(120)),
                self.disabled,
            ))
        }
    }

    fn advance(cx: &mut VisualTestContext, milliseconds: u64) -> Pixels {
        cx.executor()
            .advance_clock(Duration::from_millis(milliseconds));
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        cx.debug_bounds("face").unwrap().origin.y
    }

    #[gpui_kit::test]
    fn hover_lift_moves_smoothly_and_reverses_without_jumping(
        cx: &mut TestAppContext,
    ) {
        let (_, cx) = cx.add_window_view(|_, _| Fixture { disabled: false });
        let initial = advance(cx, 0);
        let center = cx.debug_bounds("face").unwrap().center();
        cx.simulate_mouse_move(center, None, Modifiers::none());
        assert_eq!(advance(cx, 0), initial);
        let middle = advance(cx, 60);
        assert!(middle < initial && middle > initial - px(3.));
        cx.simulate_mouse_move(point(px(0.), px(0.)), None, Modifiers::none());
        assert_eq!(advance(cx, 0), middle);
        assert_eq!(advance(cx, 120), initial);
        cx.simulate_mouse_move(center, None, Modifiers::none());
        assert_eq!(advance(cx, 120), initial - px(3.));
    }

    #[gpui_kit::test]
    fn disabled_hover_lift_keeps_the_badge_still(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| Fixture { disabled: true });
        let initial = advance(cx, 0);
        let center = cx.debug_bounds("face").unwrap().center();
        cx.simulate_mouse_move(center, None, Modifiers::none());
        assert_eq!(advance(cx, 120), initial);
    }

    #[gpui_kit::test]
    fn pressed_pointer_does_not_start_hover_motion(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| Fixture { disabled: false });
        let initial = advance(cx, 0);
        let center = cx.debug_bounds("face").unwrap().center();
        cx.simulate_mouse_move(
            center,
            Some(gpui_kit::MouseButton::Left),
            Modifiers::none(),
        );
        assert_eq!(advance(cx, 120), initial);
        cx.update(|_, cx| cx.set_reduce_motion(true));
        cx.simulate_mouse_move(center, None, Modifiers::none());
        assert_eq!(advance(cx, 120), initial);
    }
}
