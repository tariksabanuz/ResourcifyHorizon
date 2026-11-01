// The mouse: on the browser, or a press on the "+" button of the Resource Packs screen.
fn resourcify_options_mouse(
    game: &mut crate::game::Game<'_>,
    pressed: bool,
    right: bool,
) -> ::core::ops::ControlFlow<Result<()>> {
    if crate::game::native_resourcify::is_open() {
        return ::core::ops::ControlFlow::Break(crate::game::native_resourcify_ui::mouse(game, pressed, right));
    }
    if pressed && !right && crate::game::native_resourcify_ui::press_plus(game) {
        return ::core::ops::ControlFlow::Break(game.handle_option_events(vec![Event::ButtonClick]));
    }
    ::core::ops::ControlFlow::Continue(())
}
