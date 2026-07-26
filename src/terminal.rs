use anyhow::Result;
use crossterm::{cursor, execute};

pub(crate) fn with_terminal<T>(
    run: impl FnOnce(&mut ratatui::DefaultTerminal) -> Result<T>,
) -> Result<T> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);

    ratatui::restore();
    let _ = execute!(std::io::stdout(), cursor::Show);

    result
}
