//! Interactive config editor TUI: load/browse/edit/save a YAML config
//! file offline (no running collector process involved -- applying
//! changes means restarting `sgcia run` afterward).

mod app;
mod model;
mod schema_registry;
mod ui;

use app::App;
use model::EditorDoc;
use std::path::PathBuf;

pub fn run(config: PathBuf) -> anyhow::Result<()> {
    let doc = EditorDoc::load(&config).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut app = App::new(config, doc);

    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run_app(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        if app.should_quit {
            return Ok(());
        }
        if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
            app.on_key(key);
        }
    }
}
