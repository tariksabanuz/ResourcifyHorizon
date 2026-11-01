// The browser is drawn instead of the Options screen while it is open. Otherwise the Resource
// Packs screen is drawn as always, and the "+" button is drawn over it.
fn resourcify_draw_options(game: &mut crate::game::Game<'_>, batch: &mut HudBatch) -> ::core::ops::ControlFlow<()> {
    use crate::game::native_resourcify as r;
    if r::is_open() {
        crate::game::native_resourcify_ui::draw(game, batch);
        return ::core::ops::ControlFlow::Break(());
    }
    if r::DRAW.enter() {
        // This is the call made below: the original runs.
        return ::core::ops::ControlFlow::Continue(());
    }
    game.draw_options(batch);
    r::DRAW.leave();
    crate::game::native_resourcify_ui::draw_plus(game, batch);
    ::core::ops::ControlFlow::Break(())
}
