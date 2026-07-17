use crate::{
    event::{DispatchedEvent, EventDiscriminants, KeyState, ModifiersState},
    keymap::Keystroke,
    platform::keyboard::KeyCode,
};

use super::{
    AfterLayoutContext, AppContext, DispatchEventResult, Element, Event, EventContext,
    LayoutContext, PaintContext, Point, SizeConstraint, ZIndex,
};
use pathfinder_geometry::vector::Vector2F;
use std::cell::RefCell;

type Handler = Box<dyn FnMut(&mut EventContext, &AppContext, Vector2F) -> DispatchEventResult>;
type ModifiedMouseHandler = Box<
    dyn FnMut(
        &mut EventContext,
        &AppContext,
        Vector2F,
        &ModifiersState,
        u32,
    ) -> DispatchEventResult,
>;
type KeyHandler = Box<dyn FnMut(&mut EventContext, &AppContext, &Keystroke) -> DispatchEventResult>;
type ScrollHandler = Box<
    dyn FnMut(&mut EventContext, &AppContext, &Vector2F, &ModifiersState) -> DispatchEventResult,
>;
type ModifierStateChangedHandler =
    Box<dyn FnMut(&mut EventContext, &AppContext, &KeyCode, &KeyState) -> DispatchEventResult>;
type DropFilesHandler =
    Box<dyn FnMut(&mut EventContext, &AppContext, &[String], Vector2F) -> DispatchEventResult>;
type FileDragHandler =
    Box<dyn FnMut(&mut EventContext, &AppContext, Vector2F, bool) -> DispatchEventResult>;
type FileDragExitHandler = Box<dyn FnMut(&mut EventContext, &AppContext) -> DispatchEventResult>;

#[derive(Debug, Clone, Copy)]
pub struct MouseInBehavior {
    /// Whether to fire the `mouse_in` event on synthetic events, which are events the UI
    /// framework generates so in order to trigger hover effects when the underlying view has
    /// changed even though the mouse hasn't actually moved. Typically elements should handle
    /// sythetic hovers, but there are some cases where it's the incorrect behavior.
    pub fire_on_synthetic_events: bool,
    /// Whether to fire the `mouse_in` event when the element is covered by another element.
    /// This is true by default, but some elements may want to configure this behavior.
    pub fire_when_covered: bool,
}

impl Default for MouseInBehavior {
    fn default() -> Self {
        Self {
            fire_on_synthetic_events: true,
            fire_when_covered: true,
        }
    }
}

pub struct EventHandler {
    child: Box<dyn Element>,
    /// Allow this element to handle events even if a descendent already handled it.
    always_handle: bool,
    left_mouse_down: Option<RefCell<Handler>>,
    left_mouse_down_with_modifiers: Option<RefCell<ModifiedMouseHandler>>,
    left_mouse_up: Option<RefCell<Handler>>,
    middle_mouse_down: Option<RefCell<Handler>>,
    right_mouse_down: Option<RefCell<Handler>>,
    forward_mouse_down: Option<RefCell<Handler>>,
    back_mouse_down: Option<RefCell<Handler>>,
    mouse_in: Option<RefCell<Handler>>,
    mouse_in_behavior: MouseInBehavior,
    mouse_out: Option<RefCell<Handler>>,
    mouse_dragged: Option<RefCell<Handler>>,
    scroll_wheel: Option<RefCell<ScrollHandler>>,
    keydown: Option<RefCell<KeyHandler>>,
    modifier_state_changed: Option<RefCell<ModifierStateChangedHandler>>,
    /// Fired when the OS drops files onto this element's painted bounds. The
    /// callback receives the dropped paths and the drop location.
    drag_and_drop_files: Option<RefCell<DropFilesHandler>>,
    /// Fired continuously while the OS drags files over the window (before the
    /// drop). The callback receives the drag location and whether it falls
    /// inside this element's painted bounds — so a drop target can light up only
    /// while the cursor hovers it.
    file_drag: Option<RefCell<FileDragHandler>>,
    /// Fired when a file drag leaves the window entirely.
    file_drag_exit: Option<RefCell<FileDragExitHandler>>,
    origin: Option<Point>,
    // This is a short-term solution for properly handling events on stacks. A stack will always
    // put its children on higher z-indexes than its origin, so a hit test using the standard
    // `z_index` method would always result in the event being covered (by the children of the
    // stack). Instead, we track the upper-bound of z-indexes _contained by_ the child element.
    // Then we use that upper bound to do the hit testing, which means a parent will always get
    // events from its children, regardless of whether they are stacks or not.
    child_max_z_index: Option<ZIndex>,
}

impl EventHandler {
    pub fn new(child: Box<dyn Element>) -> Self {
        Self {
            child,
            always_handle: false,
            left_mouse_down: None,
            left_mouse_down_with_modifiers: None,
            left_mouse_up: None,
            middle_mouse_down: None,
            right_mouse_down: None,
            forward_mouse_down: None,
            back_mouse_down: None,
            mouse_in: None,
            mouse_out: None,
            mouse_dragged: None,
            scroll_wheel: None,
            keydown: None,
            modifier_state_changed: None,
            drag_and_drop_files: None,
            file_drag: None,
            file_drag_exit: None,
            origin: None,
            child_max_z_index: None,
            mouse_in_behavior: Default::default(),
        }
    }

    pub fn with_always_handle(mut self) -> Self {
        self.always_handle = true;
        self
    }

    pub fn on_keydown<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, &Keystroke) -> DispatchEventResult,
    {
        self.keydown = Some(RefCell::new(Box::new(callback)));
        self
    }

    pub fn on_modifier_state_changed<F>(mut self, callback: F) -> Self
    where
        F: 'static
            + FnMut(&mut EventContext, &AppContext, &KeyCode, &KeyState) -> DispatchEventResult,
    {
        self.modifier_state_changed = Some(RefCell::new(Box::new(callback)));
        self
    }

    pub fn on_left_mouse_down<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F) -> DispatchEventResult,
    {
        self.left_mouse_down = Some(RefCell::new(Box::new(callback)));
        self
    }

    /// Handles a left mouse press and exposes the modifier keys and click
    /// count carried by that mouse event. Use this for list-selection
    /// interactions where Shift extends a range, Cmd/Ctrl toggles an item,
    /// and double-click performs the primary action.
    pub fn on_left_mouse_down_with_modifiers<F>(mut self, callback: F) -> Self
    where
        F: 'static
            + FnMut(
                &mut EventContext,
                &AppContext,
                Vector2F,
                &ModifiersState,
                u32,
            ) -> DispatchEventResult,
    {
        self.left_mouse_down_with_modifiers = Some(RefCell::new(Box::new(callback)));
        self
    }

    pub fn on_left_mouse_up<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F) -> DispatchEventResult,
    {
        self.left_mouse_up = Some(RefCell::new(Box::new(callback)));
        self
    }

    pub fn on_right_mouse_down<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F) -> DispatchEventResult,
    {
        self.right_mouse_down = Some(RefCell::new(Box::new(callback)));
        self
    }

    pub fn on_middle_mouse_down<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F) -> DispatchEventResult,
    {
        self.middle_mouse_down = Some(RefCell::new(Box::new(callback)));
        self
    }

    pub fn on_forward_mouse_down<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F) -> DispatchEventResult,
    {
        self.forward_mouse_down = Some(RefCell::new(Box::new(callback)));
        self
    }

    pub fn on_back_mouse_down<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F) -> DispatchEventResult,
    {
        self.back_mouse_down = Some(RefCell::new(Box::new(callback)));
        self
    }

    pub fn on_mouse_in<F>(mut self, callback: F, mouse_in_behavior: Option<MouseInBehavior>) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F) -> DispatchEventResult,
    {
        self.mouse_in = Some(RefCell::new(Box::new(callback)));
        self.mouse_in_behavior = mouse_in_behavior.unwrap_or_default();
        self
    }

    pub fn on_mouse_out<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F) -> DispatchEventResult,
    {
        self.mouse_out = Some(RefCell::new(Box::new(callback)));
        self
    }

    pub fn on_mouse_dragged<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F) -> DispatchEventResult,
    {
        self.mouse_dragged = Some(RefCell::new(Box::new(callback)));
        self
    }

    pub fn on_scroll_wheel<F>(mut self, callback: F) -> Self
    where
        F: 'static
            + FnMut(&mut EventContext, &AppContext, &Vector2F, &ModifiersState) -> DispatchEventResult,
    {
        self.scroll_wheel = Some(RefCell::new(Box::new(callback)));
        self
    }

    /// Handle files dropped from the OS onto this element. The callback is only
    /// invoked when the drop location falls inside the element's painted bounds.
    pub fn on_drag_and_drop_files<F>(mut self, callback: F) -> Self
    where
        F: 'static
            + FnMut(&mut EventContext, &AppContext, &[String], Vector2F) -> DispatchEventResult,
    {
        self.drag_and_drop_files = Some(RefCell::new(Box::new(callback)));
        self
    }

    /// Handle the OS dragging files over the window, fired continuously before
    /// the drop. The callback receives the drag location and whether it lies
    /// within this element's painted bounds.
    pub fn on_file_drag<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F, bool) -> DispatchEventResult,
    {
        self.file_drag = Some(RefCell::new(Box::new(callback)));
        self
    }

    /// Handle a file drag leaving the window entirely.
    pub fn on_file_drag_exit<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut EventContext, &AppContext) -> DispatchEventResult,
    {
        self.file_drag_exit = Some(RefCell::new(Box::new(callback)));
        self
    }

    fn dispatch_callback(
        &self,
        callback: Option<&RefCell<Handler>>,
        ctx: &mut EventContext,
        position: Vector2F,
        app: &AppContext,
    ) -> bool {
        if let Some(callback) = callback.as_ref() {
            if let Some(rect) = ctx.visible_rect(self.origin.unwrap(), self.size().unwrap()) {
                if rect.contains_point(position) {
                    return match callback.borrow_mut()(ctx, app, position) {
                        DispatchEventResult::PropagateToParent => false,
                        DispatchEventResult::StopPropagation => true,
                    };
                }
            }
        }
        false
    }
}

impl Element for EventHandler {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        self.child.paint(origin, ctx, app);
        self.child_max_z_index = Some(ctx.scene.max_active_z_index());
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let handled = self.child.dispatch_event(event, ctx, app);
        if handled && !self.always_handle {
            return true;
        }

        let Some(z_index) = self.child_max_z_index else {
            log::error!(
                "Dispatching event {:?} on EventHandler element which was never painted.",
                EventDiscriminants::from(event.raw_event())
            );
            return false;
        };
        match event.at_z_index(z_index, ctx) {
            Some(Event::MouseMoved {
                position,
                is_synthetic,
                ..
            }) => {
                let MouseInBehavior {
                    fire_on_synthetic_events,
                    fire_when_covered,
                } = self.mouse_in_behavior;
                let is_covered = ctx.is_covered(Point::from_vec2f(
                    *position,
                    self.child_max_z_index.expect("child max z index not set"),
                ));
                let should_fire = (!is_synthetic || fire_on_synthetic_events)
                    && (fire_when_covered || !is_covered);
                if should_fire
                    && self.dispatch_callback(self.mouse_in.as_ref(), ctx, *position, app)
                {
                    return true;
                }
                if self.dispatch_callback(self.mouse_out.as_ref(), ctx, *position, app) {
                    return true;
                }
            }
            Some(Event::LeftMouseDragged { position, .. }) => {
                if self.dispatch_callback(self.mouse_dragged.as_ref(), ctx, *position, app) {
                    return true;
                }
                if self.dispatch_callback(self.mouse_in.as_ref(), ctx, *position, app) {
                    return true;
                }
                if self.dispatch_callback(self.mouse_out.as_ref(), ctx, *position, app) {
                    return true;
                }
            }
            Some(Event::LeftMouseDown {
                position,
                modifiers,
                click_count,
                ..
            }) => {
                if let Some(callback) = self.left_mouse_down_with_modifiers.as_ref() {
                    let handled =
                        match callback.borrow_mut()(ctx, app, *position, modifiers, *click_count) {
                            DispatchEventResult::PropagateToParent => false,
                            DispatchEventResult::StopPropagation => true,
                        };
                    if handled {
                        return true;
                    }
                }
                if self.dispatch_callback(self.left_mouse_down.as_ref(), ctx, *position, app) {
                    return true;
                }
            }
            Some(Event::LeftMouseUp { position, .. }) => {
                if self.dispatch_callback(self.left_mouse_up.as_ref(), ctx, *position, app) {
                    return true;
                }
            }
            Some(Event::MiddleMouseDown { position, .. }) => {
                if self.dispatch_callback(self.middle_mouse_down.as_ref(), ctx, *position, app) {
                    return true;
                }
            }
            Some(Event::RightMouseDown { position, .. }) => {
                if self.dispatch_callback(self.right_mouse_down.as_ref(), ctx, *position, app) {
                    return true;
                }
            }
            Some(Event::BackMouseDown { position, .. }) => {
                if self.dispatch_callback(self.back_mouse_down.as_ref(), ctx, *position, app) {
                    return true;
                }
            }
            Some(Event::ForwardMouseDown { position, .. }) => {
                if self.dispatch_callback(self.forward_mouse_down.as_ref(), ctx, *position, app) {
                    return true;
                }
            }
            Some(Event::KeyDown { keystroke, .. }) => {
                if let Some(callback) = self.keydown.as_ref() {
                    return match callback.borrow_mut()(ctx, app, keystroke) {
                        DispatchEventResult::PropagateToParent => false,
                        DispatchEventResult::StopPropagation => true,
                    };
                }
            }
            Some(Event::ModifierKeyChanged { key_code, state }) => {
                if let Some(callback) = self.modifier_state_changed.as_ref() {
                    return match callback.borrow_mut()(ctx, app, key_code, state) {
                        DispatchEventResult::PropagateToParent => false,
                        DispatchEventResult::StopPropagation => true,
                    };
                }
            }
            Some(Event::DragAndDropFiles { paths, location }) => {
                if let Some(callback) = self.drag_and_drop_files.as_ref() {
                    if let Some(rect) = ctx.visible_rect(self.origin.unwrap(), self.size().unwrap())
                    {
                        if rect.contains_point(*location) {
                            return match callback.borrow_mut()(ctx, app, paths, *location) {
                                DispatchEventResult::PropagateToParent => false,
                                DispatchEventResult::StopPropagation => true,
                            };
                        }
                    }
                }
            }
            Some(Event::DragFiles { location }) => {
                if let Some(callback) = self.file_drag.as_ref() {
                    let in_bounds = ctx
                        .visible_rect(self.origin.unwrap(), self.size().unwrap())
                        .is_some_and(|rect| rect.contains_point(*location));
                    return match callback.borrow_mut()(ctx, app, *location, in_bounds) {
                        DispatchEventResult::PropagateToParent => false,
                        DispatchEventResult::StopPropagation => true,
                    };
                }
            }
            Some(Event::DragFileExit) => {
                if let Some(callback) = self.file_drag_exit.as_ref() {
                    return match callback.borrow_mut()(ctx, app) {
                        DispatchEventResult::PropagateToParent => false,
                        DispatchEventResult::StopPropagation => true,
                    };
                }
            }
            Some(Event::ScrollWheel {
                position,
                delta,
                precise: _,
                modifiers,
            }) => {
                if let Some(callback) = self.scroll_wheel.as_ref() {
                    if let Some(rect) = ctx.visible_rect(self.origin.unwrap(), self.size().unwrap())
                    {
                        if rect.contains_point(*position) {
                            return match callback.borrow_mut()(ctx, app, delta, modifiers) {
                                DispatchEventResult::PropagateToParent => false,
                                DispatchEventResult::StopPropagation => true,
                            };
                        }
                    }
                }
            }
            _ => {}
        }
        handled
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }
}

#[cfg(test)]
#[path = "event_handler_tests.rs"]
mod tests;
