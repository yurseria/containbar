#![allow(deprecated, unexpected_cfgs)]

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiquidGlassRegion {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub corner_radius: f64,
    pub variant: i64,
}

#[cfg(target_os = "macos")]
mod macos {
    use super::LiquidGlassRegion;
    use cocoa::appkit::{
        NSViewHeightSizable, NSViewWidthSizable, NSVisualEffectBlendingMode,
        NSVisualEffectMaterial, NSVisualEffectState,
    };
    use cocoa::base::{id, nil, NO, YES};
    use cocoa::foundation::{NSPoint, NSRect, NSSize};
    use objc::runtime::{Class, Sel, BOOL};
    use objc::{class, msg_send, sel, sel_impl};
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};
    use tauri::{State, WebviewWindow};

    const NS_WINDOW_BELOW: i64 = -1;
    const NS_WINDOW_ABOVE: i64 = 1;
    const BACKDROP_BLUR_RADIUS: f64 = 1.5;
    const OUTER_BACKDROP_BLUR_RADIUS: f64 = 0.5;
    const BACKDROP_OVERSCAN: f64 = 48.0;
    const INNER_PANE_EXPANSION: f64 = 12.0;
    const OUTER_PANE_ADDITIONAL_EXPANSION: f64 = 12.0;
    const PANE_CORNER_RADIUS: f64 = 20.0;
    const MAX_LIQUID_GLASS_HEIGHT: f64 = 900.0;

    #[derive(Default)]
    struct WindowGlassViews {
        inner_visual_effect: usize,
        outer_visual_effect: usize,
        container: usize,
        content: usize,
        regions: HashMap<String, usize>,
    }

    #[derive(Clone, Default)]
    pub struct RegionGlassState {
        windows: Arc<Mutex<HashMap<String, WindowGlassViews>>>,
    }

    pub fn set_liquid_glass_regions(
        window: WebviewWindow,
        state: State<'_, RegionGlassState>,
        regions: Vec<LiquidGlassRegion>,
    ) -> Result<bool, String> {
        let ns_window = window
            .ns_window()
            .map_err(|error| format!("Failed to access NSWindow: {error}"))?
            as usize;
        let window_label = window.label().to_string();
        let state = state.inner().clone();

        window
            .run_on_main_thread(move || unsafe {
                sync_regions(ns_window, &window_label, &state, regions);
            })
            .map_err(|error| format!("Failed to schedule glass region update: {error}"))?;

        Ok(true)
    }

    unsafe fn sync_regions(
        ns_window_handle: usize,
        window_label: &str,
        state: &RegionGlassState,
        regions: Vec<LiquidGlassRegion>,
    ) {
        let ns_window = ns_window_handle as id;
        let window_content: id = msg_send![ns_window, contentView];
        if window_content == nil {
            return;
        }

        let mut windows = match state.windows.lock() {
            Ok(windows) => windows,
            Err(_) => return,
        };

        if regions.is_empty() {
            let _: () = msg_send![ns_window, setHasShadow: YES];
            if let Some(views) = windows.remove(window_label) {
                remove_window_views(views);
            }
            return;
        }

        if Class::get("NSVisualEffectView").is_none()
            || Class::get("NSGlassEffectView").is_none()
            || Class::get("NSGlassEffectContainerView").is_none()
        {
            return;
        }

        // The frontend auto-resizer can finish before the borderless NSPanel
        // accepts its requested size. Enforce the Liquid Glass height on the
        // native window, then let the resulting resize event resync regions
        // against the new viewport.
        let content_bounds: NSRect = msg_send![window_content, bounds];
        if (content_bounds.size.height - MAX_LIQUID_GLASS_HEIGHT).abs() > 0.5 {
            let target_size = NSSize::new(content_bounds.size.width, MAX_LIQUID_GLASS_HEIGHT);
            let _: () = msg_send![ns_window, setContentSize: target_size];
            return;
        }

        // Once the blur-only backdrop fills the window's alpha surface,
        // NSWindow's automatic shadow traces it as a rounded outer stroke.
        // Individual glass regions already provide their own depth treatment.
        let _: () = msg_send![ns_window, setHasShadow: NO];

        let views = windows.entry(window_label.to_string()).or_insert_with(|| {
            let bounds: NSRect = msg_send![window_content, bounds];
            create_window_views(window_content, bounds).unwrap_or_default()
        });

        if views.inner_visual_effect == 0
            || views.outer_visual_effect == 0
            || views.container == 0
            || views.content == 0
        {
            return;
        }

        let inner_visual_effect = views.inner_visual_effect as id;
        let outer_visual_effect = views.outer_visual_effect as id;
        let container = views.container as id;
        let glass_content = views.content as id;
        let bounds: NSRect = msg_send![window_content, bounds];
        let backdrop_frame = backdrop_frame(bounds);
        let _: () = msg_send![inner_visual_effect, setFrame: backdrop_frame];
        let _: () = msg_send![outer_visual_effect, setFrame: backdrop_frame];
        let _: () = msg_send![container, setFrame: bounds];
        let _: () = msg_send![glass_content, setFrame: bounds];
        configure_blur_only_visual_effect(inner_visual_effect, BACKDROP_BLUR_RADIUS);
        configure_blur_only_visual_effect(outer_visual_effect, OUTER_BACKDROP_BLUR_RADIUS);
        update_pane_masks(
            inner_visual_effect,
            outer_visual_effect,
            window_content,
            bounds,
            &regions,
        );

        let desired_ids: HashSet<&str> = regions
            .iter()
            .filter(|region| region.variant >= 0)
            .map(|region| region.id.as_str())
            .collect();
        let stale_ids: Vec<String> = views
            .regions
            .keys()
            .filter(|id| !desired_ids.contains(id.as_str()))
            .cloned()
            .collect();

        for id in stale_ids {
            if let Some(view) = views.regions.remove(&id) {
                let _: () = msg_send![view as id, removeFromSuperview];
            }
        }

        for region in regions {
            if region.variant < 0 {
                continue;
            }
            let Some(frame) = region_frame(bounds, &region) else {
                continue;
            };

            let view = if let Some(view) = views.regions.get(&region.id) {
                *view as id
            } else {
                let Some(view) = create_region_view(glass_content, frame) else {
                    continue;
                };
                views.regions.insert(region.id.clone(), view as usize);
                view
            };

            let _: () = msg_send![view, setFrame: frame];
            set_corner_radius(view, region.corner_radius.max(0.0));
            set_glass_variant(view, region.variant.clamp(0, 23));
            set_glass_appearance(view);
        }
    }

    unsafe fn create_window_views(content_view: id, bounds: NSRect) -> Option<WindowGlassViews> {
        let frame = backdrop_frame(bounds);
        let outer_visual_effect = create_visual_effect_view(frame)?;
        let Some(inner_visual_effect) = create_visual_effect_view(frame) else {
            let _: () = msg_send![outer_visual_effect, release];
            return None;
        };

        // The outer pane is weaker and sits below the unchanged inner pane.
        let _: () = msg_send![
            content_view,
            addSubview: outer_visual_effect
            positioned: NS_WINDOW_BELOW
            relativeTo: nil
        ];
        let _: () = msg_send![
            content_view,
            addSubview: inner_visual_effect
            positioned: NS_WINDOW_ABOVE
            relativeTo: outer_visual_effect
        ];
        configure_blur_only_visual_effect(inner_visual_effect, BACKDROP_BLUR_RADIUS);
        configure_blur_only_visual_effect(outer_visual_effect, OUTER_BACKDROP_BLUR_RADIUS);

        let container_class = Class::get("NSGlassEffectContainerView")?;
        let container: id = msg_send![container_class, alloc];
        let container: id = msg_send![container, initWithFrame: bounds];
        if container == nil {
            let _: () = msg_send![inner_visual_effect, removeFromSuperview];
            let _: () = msg_send![outer_visual_effect, removeFromSuperview];
            let _: () = msg_send![inner_visual_effect, release];
            let _: () = msg_send![outer_visual_effect, release];
            return None;
        }
        let _: () = msg_send![container, setAutoresizingMask: autoresize_mask()];
        // Keep nearby component surfaces visually independent while retaining
        // one shared container for coordinated liquid interaction.
        let _: () = msg_send![container, setSpacing: 0.0_f64];

        let content: id = msg_send![class!(NSView), alloc];
        let content: id = msg_send![content, initWithFrame: bounds];
        if content == nil {
            let _: () = msg_send![container, release];
            let _: () = msg_send![inner_visual_effect, removeFromSuperview];
            let _: () = msg_send![outer_visual_effect, removeFromSuperview];
            let _: () = msg_send![inner_visual_effect, release];
            let _: () = msg_send![outer_visual_effect, release];
            return None;
        }
        let _: () = msg_send![content, setAutoresizingMask: autoresize_mask()];
        let _: () = msg_send![container, setContentView: content];

        // Native liquid regions share one container above the blur-only
        // backdrop and below the transparent HTML controls.
        let _: () = msg_send![
            content_view,
            addSubview: container
            positioned: NS_WINDOW_ABOVE
            relativeTo: inner_visual_effect
        ];

        // The superview/container relationships retain these objects from here.
        let _: () = msg_send![content, release];
        let _: () = msg_send![container, release];
        let _: () = msg_send![inner_visual_effect, release];
        let _: () = msg_send![outer_visual_effect, release];

        Some(WindowGlassViews {
            inner_visual_effect: inner_visual_effect as usize,
            outer_visual_effect: outer_visual_effect as usize,
            container: container as usize,
            content: content as usize,
            regions: HashMap::new(),
        })
    }

    unsafe fn create_visual_effect_view(frame: NSRect) -> Option<id> {
        let visual_effect_class = Class::get("NSVisualEffectView")?;
        let visual_effect: id = msg_send![visual_effect_class, alloc];
        let visual_effect: id = msg_send![visual_effect, initWithFrame: frame];
        if visual_effect == nil {
            return None;
        }
        let _: () = msg_send![visual_effect, setAutoresizingMask: autoresize_mask()];
        let _: () = msg_send![visual_effect, setWantsLayer: YES];
        let _: () = msg_send![
            visual_effect,
            setBlendingMode: NSVisualEffectBlendingMode::BehindWindow
        ];
        let _: () = msg_send![
            visual_effect,
            setMaterial: NSVisualEffectMaterial::UnderWindowBackground
        ];
        let _: () = msg_send![visual_effect, setState: NSVisualEffectState::Active];
        Some(visual_effect)
    }

    unsafe fn create_region_view(parent: id, frame: NSRect) -> Option<id> {
        let glass_class = Class::get("NSGlassEffectView")?;
        let glass: id = msg_send![glass_class, alloc];
        let glass: id = msg_send![glass, initWithFrame: frame];
        if glass == nil {
            return None;
        }

        let _: () = msg_send![glass, setAutoresizingMask: 0_u64];
        let _: () = msg_send![glass, setWantsLayer: YES];
        let interactive_selector = Sel::register("setEffectIsInteractive:");
        let supports_interactive: BOOL = msg_send![glass, respondsToSelector: interactive_selector];
        if supports_interactive != NO {
            let _: () = msg_send![glass, setEffectIsInteractive: YES];
        }
        let _: () = msg_send![parent, addSubview: glass];
        let _: () = msg_send![glass, release];
        Some(glass)
    }

    unsafe fn remove_window_views(views: WindowGlassViews) {
        if views.inner_visual_effect != 0 {
            let _: () = msg_send![views.inner_visual_effect as id, removeFromSuperview];
        }
        if views.outer_visual_effect != 0 {
            let _: () = msg_send![views.outer_visual_effect as id, removeFromSuperview];
        }
        if views.container != 0 {
            let _: () = msg_send![views.container as id, removeFromSuperview];
        }
    }

    fn region_frame(bounds: NSRect, region: &LiquidGlassRegion) -> Option<NSRect> {
        let x = region.x.max(0.0).min(bounds.size.width);
        let top = region.y.max(0.0).min(bounds.size.height);
        let width = region.width.max(0.0).min(bounds.size.width - x);
        let height = region.height.max(0.0).min(bounds.size.height - top);
        if width < 2.0 || height < 2.0 {
            return None;
        }

        Some(NSRect::new(
            NSPoint::new(
                bounds.origin.x + x,
                bounds.origin.y + bounds.size.height - top - height,
            ),
            NSSize::new(width, height),
        ))
    }

    fn backdrop_frame(bounds: NSRect) -> NSRect {
        NSRect::new(
            NSPoint::new(
                bounds.origin.x - BACKDROP_OVERSCAN,
                bounds.origin.y - BACKDROP_OVERSCAN,
            ),
            NSSize::new(
                bounds.size.width + BACKDROP_OVERSCAN * 2.0,
                bounds.size.height + BACKDROP_OVERSCAN * 2.0,
            ),
        )
    }

    unsafe fn update_pane_masks(
        inner_visual_effect: id,
        outer_visual_effect: id,
        content_view: id,
        content_bounds: NSRect,
        regions: &[LiquidGlassRegion],
    ) {
        let inner_layer: id = msg_send![inner_visual_effect, layer];
        let outer_layer: id = msg_send![outer_visual_effect, layer];
        if inner_layer == nil || outer_layer == nil {
            return;
        }

        let mut pane_frame: Option<NSRect> = None;
        for region in regions {
            let Some(frame) = region_frame(content_bounds, region) else {
                continue;
            };
            pane_frame = Some(match pane_frame {
                Some(current) => union_rect(current, frame),
                None => frame,
            });
        }

        let Some(pane_frame) = pane_frame else {
            return;
        };
        let inner_bounds: NSRect = msg_send![inner_visual_effect, bounds];
        let outer_bounds: NSRect = msg_send![outer_visual_effect, bounds];
        let expanded_inner = expand_rect(pane_frame, INNER_PANE_EXPANSION);
        let expanded_outer = expand_rect(expanded_inner, OUTER_PANE_ADDITIONAL_EXPANSION);
        let inner_pane: NSRect = msg_send![
            inner_visual_effect,
            convertRect: expanded_inner
            fromView: content_view
        ];
        let outer_hole: NSRect = msg_send![
            outer_visual_effect,
            convertRect: expanded_inner
            fromView: content_view
        ];
        let outer_pane: NSRect = msg_send![
            outer_visual_effect,
            convertRect: expanded_outer
            fromView: content_view
        ];

        let inner_path: id = msg_send![class!(NSBezierPath), bezierPath];
        let _: () = msg_send![
            inner_path,
            appendBezierPathWithRoundedRect: inner_pane
            xRadius: PANE_CORNER_RADIUS
            yRadius: PANE_CORNER_RADIUS
        ];
        set_shape_mask(inner_layer, inner_bounds, inner_path, false);

        let outer_path: id = msg_send![class!(NSBezierPath), bezierPath];
        let _: () = msg_send![
            outer_path,
            appendBezierPathWithRoundedRect: outer_pane
            xRadius: PANE_CORNER_RADIUS + OUTER_PANE_ADDITIONAL_EXPANSION
            yRadius: PANE_CORNER_RADIUS + OUTER_PANE_ADDITIONAL_EXPANSION
        ];
        let _: () = msg_send![
            outer_path,
            appendBezierPathWithRoundedRect: outer_hole
            xRadius: PANE_CORNER_RADIUS
            yRadius: PANE_CORNER_RADIUS
        ];
        set_shape_mask(outer_layer, outer_bounds, outer_path, true);
    }

    fn expand_rect(rect: NSRect, amount: f64) -> NSRect {
        NSRect::new(
            NSPoint::new(rect.origin.x - amount, rect.origin.y - amount),
            NSSize::new(
                rect.size.width + amount * 2.0,
                rect.size.height + amount * 2.0,
            ),
        )
    }

    fn union_rect(a: NSRect, b: NSRect) -> NSRect {
        let min_x = a.origin.x.min(b.origin.x);
        let min_y = a.origin.y.min(b.origin.y);
        let max_x = (a.origin.x + a.size.width).max(b.origin.x + b.size.width);
        let max_y = (a.origin.y + a.size.height).max(b.origin.y + b.size.height);
        NSRect::new(
            NSPoint::new(min_x, min_y),
            NSSize::new(max_x - min_x, max_y - min_y),
        )
    }

    unsafe fn set_shape_mask(layer: id, bounds: NSRect, path: id, even_odd: bool) {
        let cg_path: id = msg_send![path, CGPath];
        let mask: id = msg_send![class!(CAShapeLayer), layer];
        let _: () = msg_send![mask, setFrame: bounds];
        let _: () = msg_send![mask, setPath: cg_path];
        if even_odd {
            let fill_rule = ns_string("even-odd");
            let _: () = msg_send![mask, setFillRule: fill_rule];
        }
        let _: () = msg_send![layer, setMask: mask];
    }

    unsafe fn set_corner_radius(view: id, radius: f64) {
        let selector = Sel::register("setCornerRadius:");
        let responds: BOOL = msg_send![view, respondsToSelector: selector];
        if responds != NO {
            let _: () = objc::__send_message(&*view, selector, (radius,)).unwrap_or(());
        }
    }

    unsafe fn set_glass_variant(view: id, variant: i64) {
        // Experimental material variants are what Apple's own Control Center
        // surfaces use. Match the plugin's private-then-public setter lookup.
        for selector_name in ["set_variant:", "setVariant:"] {
            let selector = Sel::register(selector_name);
            let responds: BOOL = msg_send![view, respondsToSelector: selector];
            if responds != NO {
                let _: () = objc::__send_message(&*view, selector, (variant,)).unwrap_or(());
                return;
            }
        }

        // Fall back to the public Regular/Clear style API if the experimental
        // variant selector changes in a future macOS release.
        let selector = Sel::register("setStyle:");
        let responds: BOOL = msg_send![view, respondsToSelector: selector];
        if responds != NO {
            let style = variant.clamp(0, 1);
            let _: () = objc::__send_message(&*view, selector, (style,)).unwrap_or(());
        }
    }

    unsafe fn set_glass_appearance(view: id) {
        // Let AppKit adapt the material from the sampled backdrop. Forcing a
        // DarkAqua appearance makes the glass denser and suppresses the colored
        // edge reflections that communicate lensing.
        clear_glass_tint(view);
    }

    unsafe fn clear_glass_tint(view: id) {
        let tint_selector = Sel::register("setTintColor:");
        let responds_to_tint: BOOL = msg_send![view, respondsToSelector: tint_selector];
        if responds_to_tint != NO {
            let _: () = objc::__send_message(&*view, tint_selector, (nil,)).unwrap_or(());
        }
    }

    unsafe fn configure_blur_only_visual_effect(view: id, blur_radius: f64) {
        // Keep AppKit's correctly scoped CABackdropLayer, replace its material
        // stack with one controlled Gaussian filter, and hide tint layers.
        let _: () = msg_send![view, layoutSubtreeIfNeeded];
        let _: () = msg_send![view, displayIfNeeded];
        let layer: id = msg_send![view, layer];
        let Some(backdrop_class) = Class::get("CABackdropLayer") else {
            return;
        };
        if layer != nil && install_gaussian_backdrop_filter(layer, backdrop_class, blur_radius) {
            hide_non_backdrop_layers(layer, backdrop_class);
        }
    }

    unsafe fn install_gaussian_backdrop_filter(
        layer: id,
        backdrop_class: &Class,
        blur_radius: f64,
    ) -> bool {
        let is_backdrop: BOOL = msg_send![layer, isKindOfClass: backdrop_class];
        if is_backdrop != NO {
            let filters: id = msg_send![class!(NSMutableArray), array];
            if let Some(filter_class) = Class::get("CAFilter") {
                let filter_type = ns_string("gaussianBlur");
                let gaussian: id = msg_send![filter_class, filterWithType: filter_type];
                if gaussian != nil {
                    set_filter_number(gaussian, "inputRadius", blur_radius);
                    set_filter_bool(gaussian, "inputNormalizeEdges", true);
                    let _: () = msg_send![filters, addObject: gaussian];
                }
            }

            let _: () = msg_send![layer, setFilters: filters];
            let _: () = msg_send![layer, setBackgroundColor: nil];
            let _: () = msg_send![layer, setNeedsDisplay];
            return true;
        }

        let sublayers: id = msg_send![layer, sublayers];
        if sublayers == nil {
            return false;
        }

        let count: usize = msg_send![sublayers, count];
        for index in 0..count {
            let sublayer: id = msg_send![sublayers, objectAtIndex: index];
            if install_gaussian_backdrop_filter(sublayer, backdrop_class, blur_radius) {
                return true;
            }
        }

        false
    }

    unsafe fn hide_non_backdrop_layers(layer: id, backdrop_class: &Class) {
        let is_backdrop: BOOL = msg_send![layer, isKindOfClass: backdrop_class];
        if is_backdrop != NO {
            return;
        }

        let sublayers: id = msg_send![layer, sublayers];
        if sublayers == nil {
            return;
        }

        let count: usize = msg_send![sublayers, count];
        for index in 0..count {
            let sublayer: id = msg_send![sublayers, objectAtIndex: index];
            if layer_contains_backdrop(sublayer, backdrop_class) {
                hide_non_backdrop_layers(sublayer, backdrop_class);
            } else {
                let _: () = msg_send![sublayer, setOpacity: 0.0_f32];
            }
        }
    }

    unsafe fn layer_contains_backdrop(layer: id, backdrop_class: &Class) -> bool {
        let is_backdrop: BOOL = msg_send![layer, isKindOfClass: backdrop_class];
        if is_backdrop != NO {
            return true;
        }

        let sublayers: id = msg_send![layer, sublayers];
        if sublayers == nil {
            return false;
        }

        let count: usize = msg_send![sublayers, count];
        for index in 0..count {
            let sublayer: id = msg_send![sublayers, objectAtIndex: index];
            if layer_contains_backdrop(sublayer, backdrop_class) {
                return true;
            }
        }

        false
    }

    unsafe fn ns_string(value: &str) -> id {
        let mut bytes = value.as_bytes().to_vec();
        bytes.push(0);
        msg_send![class!(NSString), stringWithUTF8String: bytes.as_ptr()]
    }

    unsafe fn set_filter_number(filter: id, key: &str, value: f64) {
        let key = ns_string(key);
        let value: id = msg_send![class!(NSNumber), numberWithDouble: value];
        let _: () = msg_send![filter, setValue: value forKey: key];
    }

    unsafe fn set_filter_bool(filter: id, key: &str, value: bool) {
        let key = ns_string(key);
        let value: id = msg_send![class!(NSNumber), numberWithBool: if value { YES } else { NO }];
        let _: () = msg_send![filter, setValue: value forKey: key];
    }

    fn autoresize_mask() -> u64 {
        NSViewWidthSizable | NSViewHeightSizable
    }
}

#[cfg(target_os = "macos")]
pub use macos::RegionGlassState;

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn set_liquid_glass_regions(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, RegionGlassState>,
    regions: Vec<LiquidGlassRegion>,
) -> Result<bool, String> {
    macos::set_liquid_glass_regions(window, state, regions)
}

#[cfg(not(target_os = "macos"))]
#[derive(Clone, Default)]
pub struct RegionGlassState;

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn set_liquid_glass_regions(
    _window: tauri::WebviewWindow,
    _state: tauri::State<'_, RegionGlassState>,
    _regions: Vec<LiquidGlassRegion>,
) -> Result<bool, String> {
    Ok(false)
}
