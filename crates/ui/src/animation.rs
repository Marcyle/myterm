use std::time::Duration;

/// 快速过渡时长，适用于 hover、按钮按压等高频交互（约 100ms）。
pub const DURATION_FAST: Duration = Duration::from_millis(100);
/// 标准过渡时长，适用于卡片、列表项状态变化（约 180ms）。
pub const DURATION_NORMAL: Duration = Duration::from_millis(180);
/// 缓慢过渡时长，适用于弹窗、面板等较大元素入场（约 280ms）。
pub const DURATION_SLOW: Duration = Duration::from_millis(280);

/// 标准缓动曲线（CSS `cubic-bezier(0.4, 0, 0.2, 1)`），用于大多数过渡。
pub fn easing_standard() -> impl Fn(f32) -> f32 {
    cubic_bezier(0.4, 0.0, 0.2, 1.0)
}

/// 减速缓动曲线（CSS `cubic-bezier(0, 0, 0.2, 1)`），用于元素入场。
pub fn easing_decelerate() -> impl Fn(f32) -> f32 {
    cubic_bezier(0.0, 0.0, 0.2, 1.0)
}

/// 加速缓动曲线（CSS `cubic-bezier(0.4, 0, 1, 1)`），用于元素退场。
pub fn easing_accelerate() -> impl Fn(f32) -> f32 {
    cubic_bezier(0.4, 0.0, 1.0, 1.0)
}

/// A cubic bezier function like CSS `cubic-bezier`.
///
/// Builder:
///
/// https://cubic-bezier.com
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> impl Fn(f32) -> f32 {
    move |t: f32| {
        let one_t = 1.0 - t;
        let one_t2 = one_t * one_t;
        let t2 = t * t;
        let t3 = t2 * t;

        // The Bezier curve function for x and y, where x0 = 0, y0 = 0, x3 = 1, y3 = 1
        let _x = 3.0 * x1 * one_t2 * t + 3.0 * x2 * one_t * t2 + t3;
        let y = 3.0 * y1 * one_t2 * t + 3.0 * y2 * one_t * t2 + t3;

        y
    }
}
