//! What a template may do, and how much a render may cost.
//!
//! A repository's templates (`.mbr/` overrides, a note's own page template)
//! are written by whoever wrote the repository, who need not be the person
//! running mbr — an app embedding mbr opens notes and templates someone else
//! sent. Every Tera engine mbr builds therefore goes through [`harden`]
//! (called by [`crate::templates::register_template_helpers`], which every
//! engine registers), and every page render through [`render_bounded`]:
//!
//! - **`get_env` is disabled.** Tera registers `get_env(name=…)` by default,
//!   which would let a template read the process environment
//!   (`AWS_SECRET_ACCESS_KEY`, tokens) into a page — and from there into an
//!   `<img src>` that sends it anywhere.
//! - **`range` is bounded.** Tera's own `range` builds the whole list
//!   (`range(end=100000000)` is 800 MB before the loop starts, and
//!   `step_by=0` never ends). The replacement has Tera's arguments and
//!   errors, refuses more than [`MAX_RANGE_LEN`] numbers in one call, a zero
//!   `step_by`, and — within a [`render_bounded`] render — more than
//!   [`MAX_RANGE_ELEMENTS_PER_RENDER`] numbers in all, so nested loops cannot
//!   multiply their way past the per-call cap.
//! - **Output is bounded.** [`render_bounded`] renders into a writer that
//!   fails once the page passes its byte limit
//!   ([`DEFAULT_TEMPLATE_OUTPUT_MAX_BYTES`], `template_output_max_bytes` in
//!   the configuration), so a render stops there instead of finishing a huge
//!   string.
//!
//! Not bounded: text a template builds in a variable without writing it
//! (`{% set_global s = s ~ s %}` in a loop) and macro bodies are buffered by
//! Tera itself, out of reach of the writer; the range budget bounds how many
//! times such a loop can run.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    hash::{Hash, Hasher},
    io::Write,
};

use tera::{Context, Tera};

/// The most numbers one `range(…)` call may produce.
pub const MAX_RANGE_LEN: usize = 100_000;

/// The most numbers all `range(…)` calls of one page render may produce
/// together (inner loops call `range` once per outer iteration).
pub const MAX_RANGE_ELEMENTS_PER_RENDER: usize = 1_000_000;

/// Default for the most bytes one rendered page may have: 16 MiB. Far above
/// any real page (mbr's own pages are tens of kilobytes), small enough that a
/// runaway template stops quickly.
pub const DEFAULT_TEMPLATE_OUTPUT_MAX_BYTES: usize = 16 * 1024 * 1024;

/// The message a template calling `get_env` gets.
const GET_ENV_DISABLED: &str = "get_env is disabled in MBR templates";

/// Makes `tera` safe for templates mbr did not write: `get_env` disabled,
/// `range` bounded (see the module docs). Idempotent; registering a
/// function replaces the previous one, so it also undoes Tera's defaults
/// whenever it runs after them.
pub fn harden(tera: &mut Tera) {
    tera.register_function("get_env", disabled_get_env);
    tera.register_function("range", bounded_range);
}

/// A fresh engine with Tera's built-ins, made safe by [`harden`]. Use this
/// (or [`crate::templates::register_template_helpers`], which calls
/// [`harden`]) wherever mbr creates an engine.
#[must_use]
pub fn new_engine() -> Tera {
    let mut tera = Tera::default();
    harden(&mut tera);
    tera
}

fn disabled_get_env(_: &HashMap<String, tera::Value>) -> tera::Result<tera::Value> {
    Err(tera::Error::msg(GET_ENV_DISABLED))
}

/// One `usize` argument of `range`, with Tera's own message for a bad one.
fn range_arg(
    args: &HashMap<String, tera::Value>,
    name: &str,
    shown: &str,
) -> tera::Result<Option<usize>> {
    args.get(name)
        .map(|value| {
            serde_json::from_value::<usize>(value.clone()).map_err(|_| {
                tera::Error::msg(format!(
                    "Function `range` received {name}={value} but `{shown}` can only be a number"
                ))
            })
        })
        .transpose()
}

/// Tera's `range(start=0, end, step_by=1)`, refusing more than
/// [`MAX_RANGE_LEN`] numbers, a zero step, and a render's range budget.
fn bounded_range(args: &HashMap<String, tera::Value>) -> tera::Result<tera::Value> {
    let start = range_arg(args, "start", "start")?.unwrap_or(0);
    let step_by = range_arg(args, "step_by", "step")?.unwrap_or(1);
    let end = range_arg(args, "end", "end")?
        .ok_or_else(|| tera::Error::msg("Function `range` was called without a `end` argument"))?;
    if start > end {
        return Err(tera::Error::msg(
            "Function `range` was called with a `start` argument greater than the `end` one",
        ));
    }
    if step_by == 0 {
        return Err(tera::Error::msg(
            "Function `range` was called with `step_by=0`, which never ends",
        ));
    }
    let len = (end - start).div_ceil(step_by);
    if len > MAX_RANGE_LEN {
        return Err(over_budget(format!(
            "Function `range` was asked for {len} numbers; MBR templates may ask for at most \
             {MAX_RANGE_LEN} in one call"
        )));
    }
    let within_budget = RANGE_BUDGET.with(|budget| match budget.get() {
        None => true,
        Some(left) if len <= left => {
            budget.set(Some(left - len));
            true
        }
        Some(_) => false,
    });
    if !within_budget {
        return Err(over_budget(format!(
            "Function `range` was called for more than {MAX_RANGE_ELEMENTS_PER_RENDER} numbers \
             in all while rendering one page; MBR templates may loop at most that much"
        )));
    }
    let numbers: Vec<tera::Value> = (start..end).step_by(step_by).map(Into::into).collect();
    Ok(tera::Value::Array(numbers))
}

thread_local! {
    /// Numbers `range` may still produce in the render running on this
    /// thread; `None` outside [`render_bounded`] (only the per-call cap
    /// applies). A Tera render runs start to finish on the calling thread.
    static RANGE_BUDGET: Cell<Option<usize>> = const { Cell::new(None) };

    /// Why the render running on this thread was stopped for its cost, set
    /// where the limit trips and read by [`render_bounded`], so the cause is
    /// known without parsing Tera's error text.
    static OVER_BUDGET: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Records `message` as the reason the current render stopped, and returns it
/// as a Tera error.
fn over_budget(message: String) -> tera::Error {
    OVER_BUDGET.with(|reason| *reason.borrow_mut() = Some(message.clone()));
    tera::Error::msg(message)
}

/// A failed [`render_bounded`].
#[derive(Debug)]
pub struct RenderFailure {
    /// Tera's error, or — when the render was stopped for its cost — one
    /// saying which limit and why.
    pub error: tera::Error,
    /// The render was stopped by a cost limit ([`MAX_RANGE_LEN`],
    /// [`MAX_RANGE_ELEMENTS_PER_RENDER`] or the output limit), not by an
    /// error in the template: rendering it again with the same input fails
    /// the same way after spending the same time.
    pub over_budget: bool,
}

/// A `Vec<u8>` that refuses to grow past `limit` bytes.
struct BoundedWriter {
    buf: Vec<u8>,
    limit: usize,
}

impl Write for BoundedWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.buf.len().saturating_add(data.len()) > self.limit {
            let message = format!(
                "the rendered page is larger than {} bytes (template_output_max_bytes); \
                 rendering stopped",
                self.limit
            );
            over_budget(message.clone());
            return Err(std::io::Error::other(message));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Renders `name` with `context` into at most `output_limit` bytes, with a
/// fresh range budget ([`MAX_RANGE_ELEMENTS_PER_RENDER`]) for this render.
///
/// # Errors
///
/// Tera's error, or one naming the limit that stopped the render
/// ([`RenderFailure::over_budget`]).
pub fn render_bounded(
    tera: &Tera,
    name: &str,
    context: &Context,
    output_limit: usize,
) -> Result<String, RenderFailure> {
    /// Restores the thread's budget state, also when the render panics.
    struct Restore(Option<usize>, Option<String>);
    impl Drop for Restore {
        fn drop(&mut self) {
            RANGE_BUDGET.with(|budget| budget.set(self.0));
            OVER_BUDGET.with(|reason| *reason.borrow_mut() = self.1.take());
        }
    }
    let _restore = Restore(
        RANGE_BUDGET.with(|budget| budget.replace(Some(MAX_RANGE_ELEMENTS_PER_RENDER))),
        OVER_BUDGET.with(|reason| reason.borrow_mut().take()),
    );

    let mut writer = BoundedWriter {
        buf: Vec::with_capacity(2000),
        limit: output_limit,
    };
    let rendered = tera.render_to(name, context, &mut writer);
    let reason = OVER_BUDGET.with(|reason| reason.borrow_mut().take());
    match (rendered, reason) {
        (Ok(()), _) => String::from_utf8(writer.buf).map_err(|e| RenderFailure {
            error: tera::Error::msg(format!("the rendered page is not UTF-8: {e}")),
            over_budget: false,
        }),
        (Err(_), Some(reason)) => Err(RenderFailure {
            error: tera::Error::msg(format!("`{name}`: {reason}")),
            over_budget: true,
        }),
        (Err(error), None) => Err(RenderFailure {
            error,
            over_budget: false,
        }),
    }
}

/// The most failures a [`CostFailureMemo`] keeps before starting over.
const COST_MEMO_CAPACITY: usize = 64;

/// Page-template renders that were stopped for their cost, so the same
/// template over the same note is refused at once instead of spending the
/// time again on every request. In memory only.
///
/// Keyed by a hash of everything the render reads that the author controls:
/// the template's path and source, its siblings, the note's rendered body and
/// frontmatter. A change to any of them is a new key, i.e. renders again.
#[derive(Debug, Default)]
pub struct CostFailureMemo {
    failures: parking_lot::Mutex<HashMap<u64, String>>,
}

impl CostFailureMemo {
    /// The key of one render; see the type docs.
    #[must_use]
    pub fn key(parts: &[&[u8]]) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for part in parts {
            part.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// The message of a remembered failure for `key`.
    #[must_use]
    pub fn get(&self, key: u64) -> Option<String> {
        self.failures.lock().get(&key).cloned()
    }

    /// Remembers that the render for `key` was stopped with `message`.
    pub fn insert(&self, key: u64, message: String) {
        let mut failures = self.failures.lock();
        if failures.len() >= COST_MEMO_CAPACITY && !failures.contains_key(&key) {
            failures.clear();
        }
        failures.insert(key, message);
    }

    /// How many failures are remembered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.failures.lock().len()
    }

    /// Whether no failure is remembered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn engine_with(source: &str) -> Tera {
        let mut tera = new_engine();
        tera.add_raw_template("t.html", source).unwrap();
        tera
    }

    fn render(source: &str, limit: usize) -> Result<String, RenderFailure> {
        render_bounded(&engine_with(source), "t.html", &Context::new(), limit)
    }

    fn message(failure: &RenderFailure) -> String {
        let mut text = failure.error.to_string();
        let mut source = std::error::Error::source(&failure.error);
        while let Some(cause) = source {
            text.push_str(&format!(": {cause}"));
            source = cause.source();
        }
        text
    }

    #[test]
    fn get_env_is_disabled() {
        let failure = render(r#"{{ get_env(name="HOME", default="x") }}"#, 1 << 20).unwrap_err();
        assert!(!failure.over_budget);
        assert!(
            message(&failure).contains("get_env is disabled"),
            "{}",
            message(&failure)
        );
    }

    #[test]
    fn range_keeps_tera_semantics() {
        let out = |s: &str| render(s, 1 << 20).unwrap();
        assert_eq!(out("{% for i in range(end=3) %}{{ i }}{% endfor %}"), "012");
        assert_eq!(
            out("{% for i in range(start=1, end=10, step_by=3) %}{{ i }},{% endfor %}"),
            "1,4,7,"
        );
        assert_eq!(out("{% for i in range(start=2, end=2) %}x{% endfor %}"), "");
        for (bad, says) in [
            ("range()", "without a `end` argument"),
            ("range(end=\"a\")", "`end` can only be a number"),
            ("range(start=-1, end=2)", "`start` can only be a number"),
            ("range(end=2, step_by=1.5)", "`step` can only be a number"),
            ("range(start=3, end=2)", "greater than the `end` one"),
            ("range(end=2, step_by=0)", "step_by=0"),
        ] {
            let failure = render(&format!("{{{{ {bad} }}}}"), 1 << 20).unwrap_err();
            assert!(
                message(&failure).contains(says),
                "{bad}: {}",
                message(&failure)
            );
            assert!(!failure.over_budget, "{bad}");
        }
    }

    #[test]
    fn a_huge_range_fails_fast_naming_range() {
        let started = Instant::now();
        let failure = render(
            "{% for i in range(end=100000000) %}{{ i }}{% endfor %}",
            DEFAULT_TEMPLATE_OUTPUT_MAX_BYTES,
        )
        .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(failure.over_budget);
        let text = message(&failure);
        assert!(
            text.contains("`range`") && text.contains("100000"),
            "{text}"
        );
    }

    #[test]
    fn nested_ranges_share_one_budget_per_render() {
        let source = "{% for i in range(end=100000) %}{% for j in range(end=100000) %}\
                      {% endfor %}{% endfor %}";
        let started = Instant::now();
        let failure = render(source, DEFAULT_TEMPLATE_OUTPUT_MAX_BYTES).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(failure.over_budget);
        assert!(
            message(&failure).contains("in all"),
            "{}",
            message(&failure)
        );
        // The budget is per render: the next render starts afresh.
        assert!(render("{% for i in range(end=100000) %}{% endfor %}", 16).is_ok());
    }

    #[test]
    fn output_over_the_limit_fails_with_the_size_error() {
        let source = "{% for i in range(end=1000) %}0123456789{% endfor %}";
        assert_eq!(render(source, 10_000).unwrap().len(), 10_000);
        let failure = render(source, 9_999).unwrap_err();
        assert!(failure.over_budget);
        let text = message(&failure);
        assert!(text.contains("larger than 9999 bytes"), "{text}");
        assert!(text.contains("template_output_max_bytes"), "{text}");
    }

    #[test]
    fn budget_state_does_not_leak_out_of_a_render() {
        let _ = render("{% for i in range(end=1000000000) %}{% endfor %}", 16);
        assert_eq!(RANGE_BUDGET.with(Cell::get), None);
        assert!(OVER_BUDGET.with(|r| r.borrow().is_none()));
    }

    #[test]
    fn memo_keys_by_content_and_stays_bounded() {
        let memo = CostFailureMemo::default();
        let a = CostFailureMemo::key(&[b"t.html", b"source", b"body"]);
        let b = CostFailureMemo::key(&[b"t.html", b"source2", b"body"]);
        assert_ne!(a, b);
        memo.insert(a, "too big".into());
        assert_eq!(memo.get(a).as_deref(), Some("too big"));
        assert_eq!(memo.get(b), None);
        for i in 0..(COST_MEMO_CAPACITY as u64 * 2) {
            memo.insert(i, String::new());
        }
        assert!(memo.len() <= COST_MEMO_CAPACITY);
    }
}
