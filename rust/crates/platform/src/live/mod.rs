//! The live display `russet run` draws in a terminal: a block for each
//! recipe in flight, a line for each of its processors, and a footer with
//! counts. Finished recipes stay in scrollback.
//!
//! Nothing here draws unless [`start`] is running. Until then every
//! function is a no-op that returns `false`, so output stays plain.
mod frame;
mod terminal;

pub use terminal::available;

use std::{
    cell::Cell,
    io::Write,
    marker::PhantomData,
    panic::PanicHookInfo,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    thread,
    time::{Duration, Instant},
};

static ACTIVE: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<frame::State> = Mutex::new(frame::State::new());
static NEXT_BLOCK: AtomicU64 = AtomicU64::new(1);
const FRAME: Duration = Duration::from_millis(80);

thread_local! {
    /// The recipe this thread is running.
    static BLOCK: Cell<Option<u64>> = const { Cell::new(None) };
}

/// Whether the display is drawing.
pub fn active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

fn state() -> MutexGuard<'static, frame::State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The block this thread writes to, while the display is drawing.
fn current() -> Option<u64> {
    if active() {
        BLOCK.with(Cell::get)
    } else {
        None
    }
}

type Hook = Box<dyn Fn(&PanicHookInfo<'_>) + Sync + Send + 'static>;

/// The running display. Dropping it draws the last frame and leaves every
/// finished recipe in scrollback.
pub struct Live {
    stop: Arc<AtomicBool>,
    ticker: Option<thread::JoinHandle<()>>,
    hook: Arc<Hook>,
    colors: frame::Colors,
}

/// Starts drawing for a run of `total` recipes.
pub fn start(total: usize) -> Live {
    state().reset(total);
    let colors = frame::Colors::from_env(
        std::env::var("NO_COLOR").ok().as_deref(),
        std::env::var("TERM").ok().as_deref(),
        std::env::var("COLORTERM").ok().as_deref(),
    );
    ACTIVE.store(true, Ordering::SeqCst);
    let hook: Arc<Hook> = Arc::new(std::panic::take_hook());
    let previous = Arc::clone(&hook);
    std::panic::set_hook(Box::new(move |info| {
        if ACTIVE.swap(false, Ordering::SeqCst) {
            let mut state = state();
            let _ = draw(&mut state, &[], "");
            state.drawn = 0;
        }
        previous(info)
    }));
    let stop = Arc::new(AtomicBool::new(false));
    let ticker = {
        let stop = Arc::clone(&stop);
        thread::Builder::new()
            .name("live-display".into())
            .spawn(move || {
                let mut tick = 0;
                while !stop.load(Ordering::SeqCst) {
                    if active() {
                        let _ = frame(tick, colors);
                    }
                    tick += 1;
                    thread::park_timeout(FRAME);
                }
            })
            .ok()
    };
    if ticker.is_none() {
        ACTIVE.store(false, Ordering::SeqCst);
    }
    Live {
        stop,
        ticker,
        hook,
        colors,
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(ticker) = self.ticker.take() {
            ticker.thread().unpark();
            let _ = ticker.join();
        }
        if ACTIVE.swap(false, Ordering::SeqCst) {
            let mut state = state();
            let size = terminal::size().unwrap_or((80, 24));
            let (committed, _) = frame::render(&mut state, size, 0, self.colors, Instant::now());
            let _ = draw(&mut state, &[], &committed);
            state.reset(0);
        }
        let _ = std::panic::take_hook();
        let hook = Arc::clone(&self.hook);
        std::panic::set_hook(Box::new(move |info| hook(info)));
    }
}

fn frame(tick: usize, colors: frame::Colors) -> std::io::Result<()> {
    let mut state = state();
    let Some(size) = terminal::size() else {
        return Ok(());
    };
    let (committed, lines) = frame::render(&mut state, size, tick, colors, Instant::now());
    draw(&mut state, &lines, &committed)
}

/// Clears the last frame, prints `committed` above the display, and draws
/// `lines`, all in one synchronized update.
fn draw(state: &mut frame::State, lines: &[String], committed: &str) -> std::io::Result<()> {
    let mut text = String::from("\x1b[?2026h\r");
    if state.drawn > 0 {
        text.push_str(&format!("\x1b[{}A", state.drawn));
    }
    text.push_str("\x1b[J");
    text.push_str(committed);
    if !committed.is_empty() && !committed.ends_with('\n') {
        text.push('\n');
    }
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    text.push_str("\x1b[?2026l");
    state.drawn = lines.len();
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(text.as_bytes())?;
    stdout.flush()
}

/// Shows a recipe until the returned guard finishes or drops. Its
/// processors and output go to its block.
pub fn recipe(name: &str) -> Block {
    let id = active().then(|| NEXT_BLOCK.fetch_add(1, Ordering::SeqCst));
    if let Some(id) = id {
        state().open(id, name, Instant::now());
    }
    let previous = BLOCK.with(|block| block.replace(id.or(block.get())));
    Block {
        id,
        previous,
        _thread_bound: PhantomData,
    }
}

/// A recipe's place in the display. Dropping it without [`Block::finish`]
/// shows the recipe as failed.
pub struct Block {
    id: Option<u64>,
    previous: Option<u64>,
    _thread_bound: PhantomData<Rc<()>>,
}

impl Block {
    /// Shows how the recipe ended. An error is shown under the recipe
    /// unless a processor already showed one.
    pub fn finish(mut self, outcome: Result<(), &str>) {
        self.close(outcome.map_err(Some));
    }

    fn close(&mut self, outcome: Result<(), Option<&str>>) {
        if let Some(id) = self.id.take() {
            let leftover = crate::text_output::take_pending();
            let mut state = state();
            for (text, stderr) in leftover {
                state.lines(Some(id), &text, stderr);
            }
            state.close(id, outcome, Instant::now());
        }
    }
}

impl Drop for Block {
    fn drop(&mut self) {
        self.close(Err(None));
        BLOCK.with(|block| block.set(self.previous));
    }
}

/// A processor in this thread's recipe started.
pub fn processor_started(name: &str) {
    if let Some(id) = current() {
        state().processor_started(id, name);
    }
}

/// This thread's current processor finished.
pub fn processor_finished() {
    if let Some(id) = current() {
        state().processor_finished(id);
    }
}

/// This thread's current processor failed. Returns whether the display
/// shows the error, so the caller doesn't print it too.
pub fn processor_failed(error: &str) -> bool {
    current().is_some_and(|id| state().processor_failed(id, error))
}

/// A processor message at `level`. Returns whether the display took it.
pub fn message(level: i64, message: &str) -> bool {
    current().is_some_and(|id| state().message(id, level, message))
}

/// Complete lines of console text. Inside a recipe they show under it;
/// otherwise they print above the display.
pub fn lines(text: &str, stderr: bool) {
    state().lines(current(), text, stderr);
}

/// How long a recipe took: tenths of a second under a minute, then whole
/// seconds with minutes and hours.
pub fn elapsed(duration: Duration) -> String {
    let tenths = duration.as_millis() / 100;
    if tenths < 600 {
        return format!("{}.{}s", tenths / 10, tenths % 10);
    }
    let seconds = duration.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours == 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{hours}h {minutes:02}m {seconds:02}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_times_use_the_largest_units() {
        let cases = [
            (0, "0.0s"),
            (420, "0.4s"),
            (1_650, "1.6s"),
            (59_999, "59.9s"),
            (60_000, "1m 00s"),
            (125_400, "2m 05s"),
            (3_600_000, "1h 00m 00s"),
            (3_725_000, "1h 02m 05s"),
        ];
        for (millis, text) in cases {
            assert_eq!(elapsed(Duration::from_millis(millis)), text, "{millis}");
        }
    }

    #[test]
    fn everything_is_a_no_op_while_inactive() {
        let block = recipe("Alpha");
        assert!(block.id.is_none());
        processor_started("FileCreator");
        assert!(!message(1, "hello"));
        assert!(!processor_failed("broken"));
        processor_finished();
        block.finish(Ok(()));
        assert!(BLOCK.with(Cell::get).is_none());
    }
}
