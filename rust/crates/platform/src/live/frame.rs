//! What the live display shows, kept apart from the terminal so it can be
//! tested: the recipes in flight, and the text each frame draws.
use std::time::Instant;

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
/// 256-color hues for the recipe spinner, in rainbow order.
const HUES: [u8; 12] = [196, 202, 214, 226, 154, 46, 49, 51, 39, 27, 93, 201];
/// ANSI colors for the recipe spinner when the terminal has only 16.
const BASIC_HUES: [u8; 6] = [31, 33, 32, 36, 34, 35];
/// The widest a header gets, so times line up in a wide window.
const HEADER_WIDTH: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colors {
    None,
    Basic,
    Extended,
}

impl Colors {
    /// Picks colors from `NO_COLOR`, `TERM`, and `COLORTERM`.
    pub fn from_env(no_color: Option<&str>, term: Option<&str>, colorterm: Option<&str>) -> Self {
        if no_color.is_some_and(|value| !value.is_empty()) {
            Colors::None
        } else if term.is_some_and(|term| term.contains("256color"))
            || colorterm.is_some_and(|value| !value.is_empty())
        {
            Colors::Extended
        } else {
            Colors::Basic
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    Plain,
    Bold,
    Dim,
    Green,
    Red,
    Yellow,
    Cyan,
    /// A step of the rainbow.
    Hue(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Running,
    Done,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NoteKind {
    /// Text a recipe wrote to stdout.
    Out,
    /// Text a recipe wrote to stderr.
    Err,
    /// The reason a processor or recipe failed.
    Error,
}

#[derive(Debug)]
struct Note {
    kind: NoteKind,
    text: String,
}

#[derive(Debug)]
struct Processor {
    name: String,
    status: Status,
    /// The processor's latest message.
    message: Option<String>,
    notes: Vec<Note>,
}

#[derive(Debug)]
struct Block {
    id: u64,
    name: String,
    started: Instant,
    /// Notes from before the first processor started.
    notes: Vec<Note>,
    processors: Vec<Processor>,
    /// Why the recipe failed, when no processor said.
    failure: Vec<Note>,
    /// Set when the recipe finishes.
    ended: Option<(Instant, bool)>,
}

#[derive(Debug)]
enum Committed {
    Block(Block),
    Text(String),
}

/// Everything the display knows about the run.
#[derive(Debug)]
pub struct State {
    blocks: Vec<Block>,
    queue: Vec<Committed>,
    total: usize,
    done: usize,
    /// How many lines the last frame left on screen.
    pub drawn: usize,
}

impl State {
    pub const fn new() -> Self {
        State {
            blocks: Vec::new(),
            queue: Vec::new(),
            total: 0,
            done: 0,
            drawn: 0,
        }
    }

    /// Starts over for a run of `total` recipes.
    pub fn reset(&mut self, total: usize) {
        *self = State::new();
        self.total = total;
    }

    pub fn open(&mut self, id: u64, name: &str, now: Instant) {
        self.blocks.push(Block {
            id,
            name: clean(name),
            started: now,
            notes: vec![],
            processors: vec![],
            failure: vec![],
            ended: None,
        });
    }

    fn block(&mut self, id: u64) -> Option<&mut Block> {
        self.blocks.iter_mut().find(|block| block.id == id)
    }

    pub fn processor_started(&mut self, id: u64, name: &str) {
        if let Some(block) = self.block(id) {
            block.processors.push(Processor {
                name: clean(name),
                status: Status::Running,
                message: None,
                notes: vec![],
            });
        }
    }

    pub fn processor_finished(&mut self, id: u64) {
        if let Some(processor) = self.block(id).and_then(|b| b.processors.last_mut()) {
            processor.status = Status::Done;
        }
    }

    /// Marks the current processor failed. Returns whether a processor took
    /// the error.
    pub fn processor_failed(&mut self, id: u64, error: &str) -> bool {
        let Some(processor) = self.block(id).and_then(|b| b.processors.last_mut()) else {
            return false;
        };
        processor.status = Status::Failed;
        processor.notes.extend(notes(NoteKind::Error, error));
        true
    }

    /// Takes a processor message. Level 0 is kept as a note, level 1
    /// becomes the processor's latest message, and higher levels are
    /// dropped. Returns whether the block took it.
    pub fn message(&mut self, id: u64, level: i64, message: &str) -> bool {
        let Some(block) = self.block(id) else {
            return false;
        };
        match level {
            ..=0 => block.add_notes(NoteKind::Out, message),
            1 => {
                if let Some(line) = message.lines().rev().find(|l| !l.trim().is_empty()) {
                    if let Some(processor) = block.processors.last_mut() {
                        processor.message = Some(clean(line.trim()));
                    }
                }
            }
            _ => {}
        }
        true
    }

    /// Takes complete lines of text. A recipe's lines become notes; lines
    /// from outside any recipe print above the display.
    pub fn lines(&mut self, id: Option<u64>, text: &str, stderr: bool) {
        let kind = if stderr { NoteKind::Err } else { NoteKind::Out };
        if let Some(block) = id.and_then(|id| self.block(id)) {
            block.add_notes(kind, text);
            return;
        }
        match self.queue.last_mut() {
            Some(Committed::Text(queued)) => queued.push_str(text),
            _ => self.queue.push(Committed::Text(text.into())),
        }
    }

    /// Finishes a recipe. A failure that no processor reported shows under
    /// the recipe, and any processor still running counts as failed.
    pub fn close(&mut self, id: u64, outcome: Result<(), Option<&str>>, now: Instant) {
        let Some(index) = self.blocks.iter().position(|block| block.id == id) else {
            return;
        };
        let mut block = self.blocks.remove(index);
        if outcome.is_err() {
            for processor in &mut block.processors {
                if processor.status == Status::Running {
                    processor.status = Status::Failed;
                }
            }
        }
        if let Err(Some(message)) = outcome {
            let reported = block
                .processors
                .iter()
                .any(|p| p.notes.iter().any(|n| n.kind == NoteKind::Error));
            if !reported {
                block.failure.extend(notes(NoteKind::Error, message));
            }
        }
        block.ended = Some((now, outcome.is_ok()));
        self.done += 1;
        self.queue.push(Committed::Block(block));
    }
}

impl Default for State {
    fn default() -> Self {
        State::new()
    }
}

impl Block {
    fn add_notes(&mut self, kind: NoteKind, text: &str) {
        let target = match self.processors.last_mut() {
            Some(processor) => &mut processor.notes,
            None => &mut self.notes,
        };
        target.extend(notes(kind, text));
    }
}

fn notes(kind: NoteKind, text: &str) -> impl Iterator<Item = Note> + '_ {
    text.lines().map(move |line| Note {
        kind,
        text: clean(line),
    })
}

/// Drops control characters, which would move the cursor, and turns tabs
/// into spaces.
fn clean(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

/// Columns a character takes. Anything outside ASCII counts as two, except
/// the symbols the display draws, so a line never runs past the window.
fn char_width(c: char) -> usize {
    if c.is_ascii() || matches!(c, '✔' | '✘' | '·' | '…' | '\u{2800}'..='\u{28ff}') {
        1
    } else {
        2
    }
}

fn text_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// Cuts `text` to `max` columns, ending with `…` when it doesn't fit.
fn truncate(text: &str, max: usize) -> String {
    if text_width(text) <= max {
        return text.into();
    }
    let mut result = String::new();
    let mut width = 0;
    for c in text.chars() {
        if width + char_width(c) + 1 > max {
            break;
        }
        width += char_width(c);
        result.push(c);
    }
    if max > 0 {
        result.push('…');
    }
    result
}

#[derive(Default)]
struct Line {
    parts: Vec<(Style, String)>,
    /// A note, which a finished recipe prints in full.
    note: bool,
}

impl Line {
    fn push(mut self, style: Style, text: impl Into<String>) -> Self {
        self.parts.push((style, text.into()));
        self
    }

    fn width(&self) -> usize {
        self.parts.iter().map(|(_, text)| text_width(text)).sum()
    }

    /// Writes the line, cut to `max` columns when there is a limit.
    fn render(&self, colors: Colors, max: Option<usize>) -> String {
        let mut room = max.unwrap_or(usize::MAX);
        let mut cut = max.is_some_and(|max| self.width() > max);
        let mut result = String::new();
        for (style, text) in &self.parts {
            if room == 0 {
                break;
            }
            let text = if cut && text_width(text) >= room {
                cut = false;
                truncate(text, room)
            } else {
                text.clone()
            };
            room = room.saturating_sub(text_width(&text));
            match code(*style, colors) {
                Some(code) if !text.is_empty() => {
                    result.push_str(&format!("\x1b[{code}m{text}\x1b[0m"))
                }
                _ => result.push_str(&text),
            }
        }
        result
    }
}

fn code(style: Style, colors: Colors) -> Option<String> {
    let code = match (style, colors) {
        (_, Colors::None) | (Style::Plain, _) => return None,
        (Style::Bold, _) => "1".into(),
        (Style::Dim, _) => "2".into(),
        (Style::Green, _) => "32".into(),
        (Style::Red, _) => "31".into(),
        (Style::Yellow, _) => "33".into(),
        (Style::Cyan, _) => "36".into(),
        (Style::Hue(step), Colors::Extended) => format!("38;5;{}", HUES[step % HUES.len()]),
        (Style::Hue(step), Colors::Basic) => BASIC_HUES[step % BASIC_HUES.len()].to_string(),
    };
    Some(code)
}

/// How a recipe's lines come out.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Detail {
    Full,
    /// Finished processors become one `✔ N done` line.
    Collapsed,
    HeaderOnly,
}

fn header(block: &Block, index: usize, tick: usize, width: usize, now: Instant) -> Line {
    let (symbol, end) = match block.ended {
        Some((end, true)) => (Line::default().push(Style::Green, "✔"), end),
        Some((end, false)) => (Line::default().push(Style::Red, "✘"), end),
        None => (
            Line::default().push(
                Style::Hue(tick + index * 3),
                SPINNER[tick % SPINNER.len()].to_string(),
            ),
            now,
        ),
    };
    let time = super::elapsed(end.saturating_duration_since(block.started));
    let target = width.saturating_sub(1).min(HEADER_WIDTH);
    let room = target.saturating_sub(2 + 2 + time.len()).max(1);
    let name = truncate(&block.name, room);
    let pad = target
        .saturating_sub(2 + text_width(&name) + time.len())
        .max(2);
    symbol
        .push(Style::Plain, " ")
        .push(Style::Bold, name)
        .push(Style::Plain, " ".repeat(pad))
        .push(Style::Dim, time)
}

fn note_line(note: &Note) -> Line {
    let style = match note.kind {
        NoteKind::Out => Style::Dim,
        NoteKind::Err => Style::Yellow,
        NoteKind::Error => Style::Red,
    };
    Line {
        note: true,
        ..Line::default()
    }
    .push(Style::Plain, "    ")
    .push(style, note.text.clone())
}

fn block_lines(
    block: &Block,
    index: usize,
    tick: usize,
    width: usize,
    now: Instant,
    detail: Detail,
) -> Vec<Line> {
    let mut lines = vec![header(block, index, tick, width, now)];
    if detail == Detail::HeaderOnly {
        return lines;
    }
    let longest = block
        .processors
        .iter()
        .map(|p| text_width(&p.name))
        .max()
        .unwrap_or(0);
    let done = block
        .processors
        .iter()
        .filter(|p| p.status == Status::Done)
        .count();
    if detail == Detail::Collapsed {
        if done > 0 {
            lines.push(
                Line::default()
                    .push(Style::Plain, "  ")
                    .push(Style::Green, "✔")
                    .push(Style::Dim, format!(" {done} done")),
            );
        }
    } else {
        lines.extend(block.notes.iter().map(note_line));
    }
    for processor in &block.processors {
        if detail == Detail::Collapsed && processor.status == Status::Done {
            continue;
        }
        let symbol = match processor.status {
            Status::Running => (Style::Cyan, SPINNER[tick % SPINNER.len()].to_string()),
            Status::Done => (Style::Green, "✔".into()),
            Status::Failed => (Style::Red, "✘".into()),
        };
        let mut line = Line::default()
            .push(Style::Plain, "  ")
            .push(symbol.0, symbol.1)
            .push(Style::Plain, " ");
        match &processor.message {
            Some(message) => {
                let pad = longest - text_width(&processor.name);
                line = line
                    .push(
                        Style::Plain,
                        format!("{}{}  ", processor.name, " ".repeat(pad)),
                    )
                    .push(Style::Dim, message.clone());
            }
            None => line = line.push(Style::Plain, processor.name.clone()),
        }
        lines.push(line);
        lines.extend(processor.notes.iter().map(note_line));
    }
    if detail == Detail::Full {
        lines.extend(block.failure.iter().map(note_line));
    }
    lines
}

/// A finished recipe's lines. Notes print in full; other lines are cut to
/// the window.
fn committed_block(block: &Block, width: usize, colors: Colors) -> String {
    let now = block.ended.map_or_else(Instant::now, |(end, _)| end);
    let limit = Some(width.saturating_sub(1));
    block_lines(block, 0, 0, width, now, Detail::Full)
        .iter()
        .map(|line| line.render(colors, if line.note { None } else { limit }) + "\n")
        .collect()
}

/// The text to print above the display, and the display's lines.
pub fn render(
    state: &mut State,
    (width, height): (usize, usize),
    tick: usize,
    colors: Colors,
    now: Instant,
) -> (String, Vec<String>) {
    let committed = state
        .queue
        .drain(..)
        .map(|item| match item {
            Committed::Text(text) => text,
            Committed::Block(block) => committed_block(&block, width, colors),
        })
        .collect();
    let max = height.saturating_sub(1);
    let footer = (state.total > 1).then(|| {
        let running = state.blocks.len();
        let waiting = state.total.saturating_sub(running + state.done);
        Line::default().push(
            Style::Dim,
            format!(
                "  {running} running · {waiting} waiting · {} done",
                state.done
            ),
        )
    });
    let reserved = usize::from(footer.is_some());
    let mut lines: Vec<Line> = vec![];
    for detail in [Detail::Full, Detail::Collapsed] {
        lines = state
            .blocks
            .iter()
            .enumerate()
            .flat_map(|(index, block)| block_lines(block, index, tick, width, now, detail))
            .collect();
        if lines.len() + reserved <= max {
            break;
        }
    }
    if lines.len() + reserved > max {
        let room = max.saturating_sub(reserved);
        let all = state.blocks.len();
        let shown = if all <= room {
            all
        } else {
            room.saturating_sub(1)
        };
        lines = state.blocks[..shown]
            .iter()
            .enumerate()
            .flat_map(|(index, block)| {
                block_lines(block, index, tick, width, now, Detail::HeaderOnly)
            })
            .collect();
        if shown < all && room > 0 {
            lines.push(Line::default().push(Style::Dim, format!("… {} more running", all - shown)));
        }
    }
    lines.extend(footer);
    lines.truncate(max);
    let limit = Some(width.saturating_sub(1));
    let live = lines
        .iter()
        .map(|line| line.render(colors, limit))
        .collect();
    (committed, live)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    /// A recipe with a finished processor and a running one.
    fn running(state: &mut State, id: u64, name: &str, start: Instant) {
        state.open(id, name, start);
        state.processor_started(id, "MozillaURLProvider");
        state.message(id, 1, "Found URL https://download.example.com/firefox");
        state.processor_finished(id);
        state.processor_started(id, "URLDownloader");
        state.message(id, 1, "Downloading\nfrom https://download.example.com");
    }

    #[test]
    fn a_running_recipe_shows_each_processor() {
        let start = Instant::now();
        let mut state = State::new();
        state.reset(1);
        running(&mut state, 1, "Firefox.munki", start);
        let (committed, lines) = render(&mut state, (100, 40), 4, Colors::None, at(start, 3_100));
        assert_eq!(committed, "");
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("⠼ Firefox.munki "), "{}", lines[0]);
        assert!(lines[0].ends_with(" 3.1s"));
        assert_eq!(text_width(&lines[0]), HEADER_WIDTH);
        assert_eq!(
            lines[1],
            "  ✔ MozillaURLProvider  Found URL https://download.example.com/firefox"
        );
        assert_eq!(
            lines[2],
            "  ⠼ URLDownloader       from https://download.example.com"
        );
    }

    #[test]
    fn long_lines_end_with_an_ellipsis() {
        let start = Instant::now();
        let mut state = State::new();
        running(&mut state, 1, &"VeryLongRecipeName".repeat(5), start);
        let (_, lines) = render(&mut state, (40, 40), 0, Colors::None, start);
        for line in &lines {
            assert!(text_width(line) <= 39, "{line}");
        }
        assert!(
            lines[0].contains('…') && lines[0].ends_with("0.0s"),
            "{}",
            lines[0]
        );
        assert!(lines[1].ends_with('…'));
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("日本語", 4), "日…");
        assert_eq!(clean("a\tb\x1b[2Jc"), "a b[2Jc");
    }

    #[test]
    fn colors_follow_the_environment() {
        assert_eq!(
            Colors::from_env(Some("1"), Some("xterm-256color"), None),
            Colors::None
        );
        assert_eq!(
            Colors::from_env(Some(""), Some("xterm-256color"), None),
            Colors::Extended
        );
        assert_eq!(
            Colors::from_env(None, Some("xterm"), Some("truecolor")),
            Colors::Extended
        );
        assert_eq!(Colors::from_env(None, Some("xterm"), None), Colors::Basic);
        let start = Instant::now();
        let mut state = State::new();
        running(&mut state, 1, "A", start);
        let (_, plain) = render(&mut state, (80, 40), 0, Colors::None, start);
        assert!(plain.iter().all(|line| !line.contains('\x1b')));
        let (_, basic) = render(&mut state, (80, 40), 0, Colors::Basic, start);
        assert!(basic[0].starts_with("\x1b[31m⠋\x1b[0m"), "{:?}", basic[0]);
        let (_, extended) = render(&mut state, (80, 40), 1, Colors::Extended, start);
        assert!(
            extended[0].starts_with("\x1b[38;5;202m⠙"),
            "{:?}",
            extended[0]
        );
        assert!(extended[1].contains("\x1b[32m✔\x1b[0m"));
    }

    #[test]
    fn a_short_window_collapses_finished_processors_then_recipes() {
        let start = Instant::now();
        let mut state = State::new();
        state.reset(8);
        running(&mut state, 1, "A", start);
        state.processor_finished(1);
        for name in ["B", "C", "D"] {
            state.processor_started(1, name);
            state.processor_finished(1);
        }
        state.processor_started(1, "E");
        let (_, lines) = render(&mut state, (80, 5), 0, Colors::None, start);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[1], "  ✔ 5 done");
        assert_eq!(lines[2], "  ⠋ E");
        assert!(lines[3].contains("1 running · 7 waiting · 0 done"));
        for id in 2..=6 {
            state.open(id, &format!("R{id}"), start);
        }
        let (_, lines) = render(&mut state, (80, 5), 0, Colors::None, start);
        assert_eq!(lines.len(), 4);
        assert!(lines[0].starts_with("⠋ A"));
        assert!(lines[1].starts_with("⠋ R2"));
        assert_eq!(lines[2], "… 4 more running");
        let (_, lines) = render(&mut state, (80, 1), 0, Colors::None, start);
        assert!(lines.is_empty());
    }

    #[test]
    fn a_finished_recipe_keeps_its_checklist() {
        let start = Instant::now();
        let mut state = State::new();
        state.reset(2);
        running(&mut state, 1, "Firefox.munki", start);
        state.lines(Some(1), "stdout text\n", false);
        state.processor_finished(1);
        state.close(1, Ok(()), at(start, 14_200));
        state.lines(None, "outside\n", false);
        let (committed, lines) = render(&mut state, (100, 40), 0, Colors::None, start);
        let committed: Vec<_> = committed.lines().collect();
        assert!(committed[0].starts_with("✔ Firefox.munki") && committed[0].ends_with("14.2s"));
        assert_eq!(
            committed[2],
            "  ✔ URLDownloader       from https://download.example.com"
        );
        assert_eq!(committed[3], "    stdout text");
        assert_eq!(committed[4], "outside");
        assert_eq!(lines, ["  0 running · 1 waiting · 1 done"]);
        let mut state = State::new();
        running(&mut state, 1, "A", start);
        state.lines(Some(1), &format!("{}\n", "n".repeat(60)), true);
        state.close(1, Ok(()), start);
        let (committed, _) = render(&mut state, (40, 40), 0, Colors::None, start);
        let committed: Vec<_> = committed.lines().collect();
        assert!(committed[1].ends_with('…') && text_width(committed[1]) == 39);
        assert_eq!(committed[3], format!("    {}", "n".repeat(60)));
    }

    #[test]
    fn failures_show_once() {
        let start = Instant::now();
        let mut state = State::new();
        running(&mut state, 1, "A", start);
        assert!(state.processor_failed(1, "No such file"));
        state.close(1, Err(Some("Error in A: No such file")), start);
        running(&mut state, 2, "B", start);
        state.close(2, Err(Some("Bad input")), start);
        state.open(3, "C", start);
        assert!(!state.processor_failed(3, "too early"));
        state.close(3, Err(None), start);
        let (committed, _) = render(&mut state, (80, 40), 0, Colors::None, start);
        let committed: Vec<_> = committed.lines().collect();
        assert!(committed[0].starts_with("✘ A"));
        assert_eq!(
            committed[2],
            "  ✘ URLDownloader       from https://download.example.com"
        );
        assert_eq!(committed[3], "    No such file");
        assert!(committed[4].starts_with("✘ B"));
        assert_eq!(
            committed[6],
            "  ✘ URLDownloader       from https://download.example.com"
        );
        assert_eq!(committed[7], "    Bad input");
        assert!(committed[8].starts_with("✘ C"));
        assert_eq!(committed.len(), 9);
    }

    #[test]
    fn messages_by_level() {
        let start = Instant::now();
        let mut state = State::new();
        assert!(!state.message(1, 0, "no block"));
        state.open(1, "A", start);
        assert!(state.message(1, 0, "before any processor"));
        state.processor_started(1, "P");
        assert!(state.message(1, 2, "setting default value"));
        assert!(state.message(1, 0, "kept"));
        let (_, lines) = render(&mut state, (80, 40), 0, Colors::None, start);
        assert_eq!(
            lines[1..],
            ["    before any processor", "  ⠋ P", "    kept"]
        );
    }
}
