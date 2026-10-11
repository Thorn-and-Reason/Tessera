use crate::{
    data::RevisionId,
    error::{AppError, AppResult},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
    Arc, Mutex,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RunId(pub u64);
impl RunId {
    pub fn next(self) -> AppResult<Self> {
        if self.0 == 0 {
            return Err(AppError::IdentityExhausted);
        }
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(AppError::IdentityExhausted)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobToken {
    pub document: u64,
    pub generation: u64,
    pub revision: RevisionId,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: Option<usize>,
}
#[derive(Debug, Clone, Default)]
pub struct JobControl {
    cancellation: Arc<AtomicBool>,
    progress: Option<Sender<Progress>>,
    latest: Option<Arc<Mutex<Option<Progress>>>>,
}
impl JobControl {
    // Default cancellation Arc: two reference counts plus an aligned AtomicBool.
    pub(crate) const DEFAULT_HEAP_BYTES: usize = 3 * std::mem::size_of::<usize>();
    pub fn new(cancellation: Arc<AtomicBool>, progress: Option<Sender<Progress>>) -> Self {
        Self {
            cancellation,
            progress,
            latest: None,
        }
    }
    pub(crate) fn coalesced(latest: Arc<Mutex<Option<Progress>>>) -> Self {
        Self {
            latest: Some(latest),
            ..Self::default()
        }
    }
    pub fn cancel(&self) {
        self.cancellation.store(true, Ordering::Release);
    }
    pub(crate) fn cancellation_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancellation)
    }
    /// Reusable helper checks share cancellation without publishing local progress.
    pub(crate) fn without_progress(&self) -> Self {
        Self {
            cancellation: Arc::clone(&self.cancellation),
            progress: None,
            latest: None,
        }
    }
    pub fn check_cancelled(&self) -> AppResult<()> {
        if self.cancellation.load(Ordering::Acquire) {
            Err(AppError::Cancelled)
        } else {
            Ok(())
        }
    }
    pub fn checkpoint(&self, done: usize, total: Option<usize>) -> AppResult<()> {
        #[cfg(test)]
        CHECKPOINT_HOOK.with(|hook| {
            if let Some(hook) = hook.borrow().as_ref() {
                hook(Progress { done, total });
            }
        });
        self.check_cancelled()?;
        if let Some(sender) = &self.progress {
            // A dropped receiver means the interface no longer needs progress.
            let _ = sender.send(Progress { done, total });
        }
        if let Some(latest) = &self.latest {
            *latest.lock().unwrap_or_else(|error| error.into_inner()) =
                Some(Progress { done, total });
        }
        Ok(())
    }
}

#[cfg(test)]
type CheckpointHook = Box<dyn Fn(Progress)>;
#[cfg(test)]
thread_local! {
    pub(crate) static CHECKPOINT_HOOK: std::cell::RefCell<Option<CheckpointHook>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_control_heap_owner_matches_its_allocation() {
        let (control, largest) = crate::data::measure_test_allocations(JobControl::default);
        assert_eq!(largest, JobControl::DEFAULT_HEAP_BYTES);
        drop(control);
    }
    #[test]
    fn neutral_view_shares_cancellation_and_hooks_without_sender_or_slot_updates() {
        use std::{cell::Cell, rc::Rc};
        let (sender, receiver) = std::sync::mpsc::channel();
        let control = JobControl::new(Arc::new(AtomicBool::new(false)), Some(sender));
        control.checkpoint(7, Some(10)).unwrap();
        let quiet = control.without_progress();
        let hooks = Rc::new(Cell::new(0));
        let observed = Rc::clone(&hooks);
        CHECKPOINT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |p| {
                assert_eq!(p.done, 0);
                observed.set(observed.get() + 1);
            }))
        });
        quiet.checkpoint(0, None).unwrap();
        CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
        assert_eq!(hooks.get(), 1);
        assert_eq!(
            receiver.try_iter().collect::<Vec<_>>(),
            vec![Progress {
                done: 7,
                total: Some(10)
            }]
        );
        quiet.cancel();
        assert!(matches!(
            control.check_cancelled(),
            Err(AppError::Cancelled)
        ));
        let latest = Arc::new(Mutex::new(None));
        let control = JobControl::coalesced(Arc::clone(&latest));
        control.checkpoint(9, Some(12)).unwrap();
        let quiet = control.without_progress();
        quiet.checkpoint(0, None).unwrap();
        assert_eq!(
            *latest.lock().unwrap(),
            Some(Progress {
                done: 9,
                total: Some(12)
            })
        );
        control.cancel();
        assert!(matches!(quiet.check_cancelled(), Err(AppError::Cancelled)));
    }
    #[test]
    fn neutral_cancellation_preserves_coalesced_progress() {
        let latest = Arc::new(Mutex::new(None));
        let control = JobControl::coalesced(Arc::clone(&latest));
        control.checkpoint(7, Some(10)).unwrap();
        control.check_cancelled().unwrap();
        assert_eq!(
            *latest.lock().unwrap(),
            Some(Progress {
                done: 7,
                total: Some(10)
            })
        );
        control.cancel();
        assert!(matches!(
            control.check_cancelled(),
            Err(AppError::Cancelled)
        ));
        assert_eq!(
            *latest.lock().unwrap(),
            Some(Progress {
                done: 7,
                total: Some(10)
            })
        );
    }
}
