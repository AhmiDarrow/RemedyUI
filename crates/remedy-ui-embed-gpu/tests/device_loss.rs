//! A lost GPU device is replaced and embedded content follows it. Its own process: losing
//! the shared device would disturb other tests running beside it.

mod common;

use common::*;
use remedy_ui_render_gpu::GpuContext;

#[test]
fn a_lost_device_is_replaced_and_the_content_prepared_again() {
    let (mut s, _el, view, _angle, seen) = triangle(true);
    let ctx = GpuContext::current_shared().unwrap_or_else(|| unreachable!("the shared device"));
    ctx.force_loss();
    // The window's compositor recovers onto the replacement; the content follows.
    if let Renderer::Gpu(g) = &mut s.renderer {
        assert!(g.recover(&mut s.comp).is_ok());
    }
    view.request_redraw();
    s.frame();
    assert_eq!(seen.prepared.get(), 2, "prepared again on the new device");
    assert_eq!(view.stats().allocations, 2);
    assert!(near(s.px(100, 30), RED), "{:?}", s.px(100, 30));
    let now = GpuContext::current_shared().map(|c| c.device_serial());
    assert_ne!(now, Some(ctx.device_serial()));
}
