//! Work to do once a message is handled: futures that answer with more
//! messages, and what the editor is asked (a tab opened, the clipboard).
//! The same shape as the tasks of the editor's interface, so the logic of
//! the Database tool reads the same.

use std::future::Future;
use std::pin::Pin;

type Boxed<M> = Pin<Box<dyn Future<Output = Option<M>> + Send>>;

/// Something the editor is asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Opens (or brings to the front) a tab of the plugin: its id, its title.
    OpenTab(String, String),
    CloseTab(String),
    Copy(String),
    /// Says something: the text, and whether it is an error.
    Say(String, bool),
    /// Shows a panel of the plugin.
    ShowPanel(String),
}

pub enum Task<M> {
    None,
    Future(Boxed<M>),
    Effect(Effect),
    Batch(Vec<Task<M>>),
}

impl<M: Send + 'static> Task<M> {
    pub fn none() -> Self {
        Task::None
    }
    pub fn effect(effect: Effect) -> Self {
        Task::Effect(effect)
    }

    pub fn perform<T, F>(future: F, then: impl FnOnce(T) -> M + Send + 'static) -> Self
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        Task::Future(Box::pin(async move { Some(then(future.await)) }))
    }
    /// The same task, and a handle that stops it: its future is dropped
    /// (and the driver it was waiting for with it).
    pub fn abortable(self) -> (Self, Handle) {
        let handle = Handle::default();
        let stop = handle.0.clone();
        let task = match self {
            Task::Future(future) => Task::Future(Box::pin(async move {
                tokio::select! {
                    answer = future => answer,
                    _ = stop.notified() => None,
                }
            })),
            other => other,
        };
        (task, handle)
    }

    pub fn batch(tasks: impl IntoIterator<Item = Task<M>>) -> Self {
        let tasks: Vec<_> = tasks
            .into_iter()
            .filter(|t| !matches!(t, Task::None))
            .collect();
        match tasks.len() {
            0 => Task::None,
            _ => Task::Batch(tasks),
        }
    }
    /// The effects to carry out now, and the futures to run.
    pub fn split(self, effects: &mut Vec<Effect>, futures: &mut Vec<Boxed<M>>) {
        match self {
            Task::None => {}
            Task::Effect(e) => effects.push(e),
            Task::Future(f) => futures.push(f),
            Task::Batch(tasks) => {
                for t in tasks {
                    t.split(effects, futures);
                }
            }
        }
    }
}

/// Stops a task made [`Task::abortable`].
#[derive(Debug, Clone, Default)]
pub struct Handle(std::sync::Arc<tokio::sync::Notify>);

impl Handle {
    pub fn abort(&self) {
        self.0.notify_one();
    }
}

/// Writes to the clipboard (as the editor's own task does).
pub fn copy<M: Send + 'static>(text: impl Into<String>) -> Task<M> {
    Task::Effect(Effect::Copy(text.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_keeps_what_is_ready_apart_from_what_waits() {
        let task: Task<u32> = Task::batch([
            Task::none(),
            Task::perform(async { 2 }, |n| (n + 1) * 10),
            copy("x"),
        ]);
        let (mut effects, mut futures) = (Vec::new(), Vec::new());
        task.split(&mut effects, &mut futures);
        assert_eq!(effects, vec![Effect::Copy("x".into())]);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        assert_eq!(runtime.block_on(futures.pop().unwrap()), Some(30));
    }
}
