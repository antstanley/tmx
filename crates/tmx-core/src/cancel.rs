//! Cancellation — the root-threaded token that stops in-flight work on `--timeout` or SIGINT.
//!
//! A single [`CancelToken`] is threaded from the root into every adapter call (through the [`Ports`]
//! bundle) and awaited **alongside** the work ([06 §Concurrency, cancellation,
//! timeouts](../../../.specs/06-ports-and-adapters.md); [08 §Cancellation, timeout,
//! interrupt](../../../.specs/08-errors-and-observability.md)). The contract is two-phase:
//!
//! - **Requested** (soft) — set by `--timeout` (via the `Clock`) or SIGINT at the `main` seam. The
//!   sequential runner (the degenerate `Scheduler`) reads [`CancelToken::requested_reason`] at the top
//!   of its loop and **stops dispatching new work**; the in-flight adapter keeps running.
//! - **Hard** — set after the [`CANCEL_GRACE_MS`](tmx_schema::limits::CANCEL_GRACE_MS) grace window
//!   (overridable via `--grace`). [`CancelToken::guard`] wraps each in-flight adapter await and, once
//!   hard cancellation fires, resolves to a typed [`RunError`] — dropping the work future, so an
//!   adapter that ignores the grace period is **hard-stopped** and no cancelled run is held hostage.
//!
//! The token is **pure**: it is a shared flag plus a waker list backed by `std::sync::Mutex`, with no
//! async-runtime edge (no `tokio`), so it stays inside the core's purity boundary. The real timer and
//! signal that *trigger* it live in the CLI driving adapter, where the async runtime does.
//!
//! A never-triggered token ([`CancelToken::new`] / [`Default`]) is a total no-op: [`guard`] simply
//! yields the work's result, so every normal run threads the token yet behaves exactly as before.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

use crate::error::{ErrorCategory, RunError};
use crate::model::RunStatus;

/// Why a run was cancelled — the closed two-value vocabulary that maps to the terminal status and the
/// error category (and, at the `main` seam, the exit code 124 / 130).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    /// The run exceeded its `--timeout` budget (ends `timed_out`, category `Timeout`, exit 124).
    Timeout,
    /// The run was interrupted by SIGINT (ends `cancelled`, category `Interrupt`, exit 130).
    Interrupt,
}

impl CancelReason {
    /// The stable lower-case token for this reason (`timeout` / `interrupt`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            CancelReason::Timeout => "timeout",
            CancelReason::Interrupt => "interrupt",
        }
    }

    /// The terminal [`RunStatus`] a run cancelled for this reason ends in.
    #[must_use]
    pub const fn to_status(self) -> RunStatus {
        match self {
            CancelReason::Timeout => RunStatus::TimedOut,
            CancelReason::Interrupt => RunStatus::Cancelled,
        }
    }

    /// The typed [`RunError`] a hard cancellation for this reason surfaces from [`CancelToken::guard`].
    #[must_use]
    pub fn to_error(self) -> RunError {
        match self {
            CancelReason::Timeout => RunError::new(
                ErrorCategory::Timeout,
                "timeout",
                "the run exceeded its --timeout budget and was cancelled",
            ),
            CancelReason::Interrupt => RunError::new(
                ErrorCategory::Interrupt,
                "interrupt",
                "the run was interrupted (SIGINT) and cancelled",
            ),
        }
    }
}

/// The shared, lock-guarded cancellation state: the two phase flags plus the waker list that lets a
/// [`Guard`] parked on the token be re-polled when hard cancellation fires.
#[derive(Debug, Default)]
struct Shared {
    /// The soft-cancel reason (stop dispatching new work), once requested.
    requested: Option<CancelReason>,
    /// The hard-cancel reason (abandon in-flight work), once the grace window has elapsed.
    hard: Option<CancelReason>,
    /// Wakers of futures parked awaiting hard cancellation; drained and woken when it fires.
    wakers: Vec<Waker>,
}

/// A cheaply-cloned handle to one run's cancellation state.
///
/// Clones share the same underlying flag (an `Arc`), so the CLI can hold one clone to *trigger*
/// cancellation from a timer / signal task while the runner holds another (through [`Ports`]) to
/// *observe* it. `Default`/[`CancelToken::new`] is a never-triggered token — a total no-op.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    inner: Arc<Mutex<Shared>>,
}

impl CancelToken {
    /// A fresh, never-triggered token. Threading it changes nothing until [`request`](Self::request)
    /// or [`hard_cancel`](Self::hard_cancel) fires.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Lock the shared state, recovering the guard from a poisoned lock rather than panicking (a
    /// poisoned cancel flag is still a valid flag — the worst a panicked holder left is a set/unset
    /// bool, never a torn value).
    fn lock(&self) -> MutexGuard<'_, Shared> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Request cancellation (the soft phase): the runner stops dispatching **new** work, but in-flight
    /// adapters keep running until the grace window elapses and [`hard_cancel`](Self::hard_cancel)
    /// fires. The first reason wins — a later request does not overwrite it.
    pub fn request(&self, reason: CancelReason) {
        let mut shared = self.lock();
        if shared.requested.is_none() {
            shared.requested = Some(reason);
        }
    }

    /// Escalate to a hard stop: in-flight [`guard`](Self::guard)ed work is abandoned at the next poll.
    /// Implies [`request`](Self::request), and wakes every parked [`Guard`] so a runtime re-polls it
    /// promptly. The first reason wins for both phases.
    pub fn hard_cancel(&self, reason: CancelReason) {
        let mut shared = self.lock();
        if shared.requested.is_none() {
            shared.requested = Some(reason);
        }
        if shared.hard.is_none() {
            shared.hard = Some(reason);
        }
        let wakers = std::mem::take(&mut shared.wakers);
        drop(shared);
        for waker in wakers {
            waker.wake();
        }
    }

    /// The soft-cancel reason, if cancellation has been requested — the runner's stop-dispatching gate.
    #[must_use]
    pub fn requested_reason(&self) -> Option<CancelReason> {
        self.lock().requested
    }

    /// The hard-cancel reason, if the grace window has elapsed — the abandon-in-flight signal.
    #[must_use]
    pub fn hard_reason(&self) -> Option<CancelReason> {
        self.lock().hard
    }

    /// Park `waker` to be woken when hard cancellation fires. A no-op duplicate (`will_wake`) is not
    /// re-parked, so a repeatedly-polled guard does not grow the list without bound; if hard
    /// cancellation has *already* fired, the waker is woken immediately rather than parked.
    fn register(&self, waker: &Waker) {
        let mut shared = self.lock();
        if shared.hard.is_some() {
            drop(shared);
            waker.wake_by_ref();
            return;
        }
        if !shared.wakers.iter().any(|parked| parked.will_wake(waker)) {
            shared.wakers.push(waker.clone());
        }
    }

    /// Await `work` **alongside** this token: resolve to the work's result when it finishes first, or
    /// to a typed cancellation [`RunError`] the moment hard cancellation fires — dropping (hard-
    /// stopping) the in-flight `work` future. A never-triggered token makes this a transparent pass-
    /// through of `work`.
    #[must_use = "the guarded future does nothing unless awaited"]
    pub fn guard<'a, T, F>(&'a self, work: F) -> Guard<'a, T>
    where
        F: Future<Output = Result<T, RunError>> + Send + 'a,
    {
        Guard {
            token: self,
            work: Box::pin(work),
        }
    }
}

/// The future returned by [`CancelToken::guard`]: races `work` against hard cancellation of the token.
///
/// Both fields are `Unpin` (a shared reference and a boxed future), so the future is `Unpin` and its
/// `poll` can take `&mut self` without any `unsafe` pin projection — the projection-free discipline
/// `#![forbid(unsafe_code)]` requires.
#[must_use = "a Guard does nothing unless awaited"]
pub struct Guard<'a, T> {
    token: &'a CancelToken,
    work: Pin<Box<dyn Future<Output = Result<T, RunError>> + Send + 'a>>,
}

impl<T> Future for Guard<'_, T> {
    type Output = Result<T, RunError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let guard = self.get_mut();
        // Hard cancellation wins over the work: if it has already fired, abandon the work future
        // (dropped when this future is) and surface the typed cancellation error.
        if let Some(reason) = guard.token.hard_reason() {
            return Poll::Ready(Err(reason.to_error()));
        }
        match guard.work.as_mut().poll(cx) {
            Poll::Ready(value) => Poll::Ready(value),
            Poll::Pending => {
                // Park on the token so a later hard cancel re-polls us, and re-check to close the race
                // where hard cancellation fired between the poll above and this registration.
                guard.token.register(cx.waker());
                if let Some(reason) = guard.token.hard_reason() {
                    return Poll::Ready(Err(reason.to_error()));
                }
                Poll::Pending
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::Wake;

    /// A waker that records whether it was woken — enough to prove `hard_cancel` wakes a parked guard.
    struct FlagWaker(AtomicBool);
    impl Wake for FlagWaker {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::SeqCst);
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn reason_maps_to_status_category_and_token() {
        // Each reason maps 1:1 to a terminal status, an error category, and its wire token — the
        // mapping the runner and the `main` exit-code seam both rely on.
        assert_eq!(CancelReason::Timeout.to_status(), RunStatus::TimedOut);
        assert_eq!(CancelReason::Interrupt.to_status(), RunStatus::Cancelled);
        assert_eq!(
            CancelReason::Timeout.to_error().category,
            ErrorCategory::Timeout,
            "a timeout is the Timeout category (→ exit 124)"
        );
        assert_eq!(
            CancelReason::Interrupt.to_error().category,
            ErrorCategory::Interrupt,
            "an interrupt is the Interrupt category (→ exit 130)"
        );
        assert_eq!(CancelReason::Timeout.as_str(), "timeout");
        assert_eq!(CancelReason::Interrupt.as_str(), "interrupt");
    }

    #[test]
    fn two_phase_flags_are_first_write_wins_and_hard_implies_requested() {
        let token = CancelToken::new();
        assert!(
            token.requested_reason().is_none() && token.hard_reason().is_none(),
            "a fresh token is fully un-cancelled"
        );

        token.request(CancelReason::Timeout);
        assert_eq!(
            token.requested_reason(),
            Some(CancelReason::Timeout),
            "request sets the soft phase"
        );
        assert!(
            token.hard_reason().is_none(),
            "request alone does not hard-cancel — the in-flight work keeps its grace window"
        );

        // A later request does not overwrite the first requested reason.
        token.request(CancelReason::Interrupt);
        assert_eq!(
            token.requested_reason(),
            Some(CancelReason::Timeout),
            "the first requested reason wins"
        );

        // Escalating to a hard stop (a watcher uses one reason for both phases) sets the hard phase.
        token.hard_cancel(CancelReason::Timeout);
        assert_eq!(
            token.hard_reason(),
            Some(CancelReason::Timeout),
            "hard_cancel sets the hard phase"
        );

        // A separate token proves hard_cancel *implies* requested when nothing requested it first.
        let direct = CancelToken::new();
        direct.hard_cancel(CancelReason::Interrupt);
        assert_eq!(
            direct.requested_reason(),
            Some(CancelReason::Interrupt),
            "a bare hard_cancel implies the soft phase too (stop dispatching, then abandon)"
        );
        assert_eq!(direct.hard_reason(), Some(CancelReason::Interrupt));
    }

    /// Poll `fut` once with a plain no-op-context waker.
    fn poll_once<F: Future + Unpin>(fut: &mut F, waker: &Waker) -> Poll<F::Output> {
        let mut cx = Context::from_waker(waker);
        Pin::new(fut).poll(&mut cx)
    }

    #[test]
    fn guard_passes_through_a_ready_work_result_when_never_cancelled() {
        // Regression: a never-triggered token is a transparent pass-through — a ready future's Ok
        // value comes straight back, so every normal run is unaffected by threading the token.
        let token = CancelToken::new();
        let waker = Waker::from(Arc::new(FlagWaker(AtomicBool::new(false))));
        let mut guarded = token.guard(async { Ok::<u32, RunError>(7) });
        match poll_once(&mut guarded, &waker) {
            Poll::Ready(result) => {
                assert_eq!(result.expect("ok"), 7, "the work result passes through")
            }
            Poll::Pending => panic!("a ready future must complete on the first poll"),
        }
    }

    #[test]
    fn guard_hard_stops_pending_work_at_the_deadline_and_wakes_the_parked_poller() {
        // O2 in miniature: work that never returns (an adapter ignoring the grace period) is
        // hard-stopped when hard cancellation fires — the guard resolves to the typed cancellation
        // error and the parked waker is woken so a runtime re-polls promptly.
        let token = CancelToken::new();
        let flag = Arc::new(FlagWaker(AtomicBool::new(false)));
        let waker = Waker::from(Arc::clone(&flag));

        let mut guarded = token.guard(std::future::pending::<Result<u32, RunError>>());
        // First poll: the work is pending and no cancellation yet, so the guard parks.
        assert!(
            matches!(poll_once(&mut guarded, &waker), Poll::Pending),
            "the guard is pending while the work hangs and no cancel has fired"
        );
        assert!(
            !flag.0.load(Ordering::SeqCst),
            "no premature wake before cancellation"
        );

        // The grace window elapses → hard cancel. The parked waker must be woken.
        token.hard_cancel(CancelReason::Timeout);
        assert!(
            flag.0.load(Ordering::SeqCst),
            "hard_cancel wakes the parked guard so the runtime re-polls it"
        );

        // Re-poll: the still-hanging work is abandoned and the typed cancellation error surfaces.
        match poll_once(&mut guarded, &waker) {
            Poll::Ready(result) => {
                let err = result.expect_err("a hard-cancelled guard yields the cancellation error");
                assert_eq!(
                    err.category,
                    ErrorCategory::Timeout,
                    "the hard stop surfaces the reason's category"
                );
                assert_eq!(err.code, "timeout", "with its stable code");
            }
            Poll::Pending => panic!("a hard-cancelled guard must resolve, not stay hostage"),
        }
    }

    #[test]
    fn guard_observes_a_cancel_that_fired_before_the_first_poll() {
        // Negative space around the ordering: if hard cancellation already fired before the guard is
        // ever polled, the very first poll resolves to the cancellation error rather than touching the
        // work — no window in which a hard-cancelled guard runs its work.
        let token = CancelToken::new();
        token.hard_cancel(CancelReason::Interrupt);
        let waker = Waker::from(Arc::new(FlagWaker(AtomicBool::new(false))));
        let mut guarded = token.guard(std::future::pending::<Result<u32, RunError>>());
        match poll_once(&mut guarded, &waker) {
            Poll::Ready(result) => assert_eq!(
                result.expect_err("already-cancelled").category,
                ErrorCategory::Interrupt,
                "a pre-fired hard cancel resolves on the first poll"
            ),
            Poll::Pending => panic!("an already-hard-cancelled guard resolves immediately"),
        }
    }
}
