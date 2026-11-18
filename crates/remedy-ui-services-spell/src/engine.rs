//! The checking thread: owns the back ends, answers requests, and checks long texts in slices
//! so a request for suggestions never waits behind a whole message.

use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use crate::SpellError;
use crate::backend::{Availability, BackendFactory, BackendReport, Prepared, SpellBackend};
use crate::languages::in_script_of;
use crate::personal::PersonalDictionary;
use crate::words::find_words;

/// How many words a check looks at before it listens for other requests.
const SLICE: usize = 32;

/// Something to do on the checking thread.
pub(crate) enum Job {
    Check {
        text: String,
        languages: Vec<String>,
        cancel: Arc<AtomicBool>,
        reply: Sender<Result<Vec<Range<usize>>, SpellError>>,
        notify: Option<Notify>,
    },
    Word {
        word: String,
        languages: Vec<String>,
        reply: Sender<Result<bool, SpellError>>,
    },
    Suggest {
        word: String,
        languages: Vec<String>,
        reply: Sender<Result<Vec<String>, SpellError>>,
    },
    Availability {
        language: String,
        reply: Sender<Availability>,
    },
    Report {
        reply: Sender<Vec<BackendReport>>,
    },
    Languages {
        reply: Sender<Vec<String>>,
    },
}

/// What to call once a check's answer is waiting.
pub(crate) type Notify = Box<dyn FnOnce() + Send>;

type CheckReply = Sender<Result<Vec<Range<usize>>, SpellError>>;

/// Sends a check's answer, then tells whoever wanted to know.
fn answer(reply: &CheckReply, notify: &mut Option<Notify>, result: Result<Vec<Range<usize>>, SpellError>) {
    let _ = reply.send(result);
    if let Some(n) = notify.take() {
        n();
    }
}

/// A language and the back end that checks it.
#[derive(Clone, Debug)]
struct Usable {
    language: String,
    backend: usize,
}

/// A check in progress.
struct Active {
    words: Vec<Range<usize>>,
    next: usize,
    found: Vec<Range<usize>>,
    text: String,
    usable: Vec<Usable>,
    cancel: Arc<AtomicBool>,
    reply: CheckReply,
    notify: Option<Notify>,
}

struct Slot {
    backend: Box<dyn SpellBackend>,
}

pub(crate) struct Engine {
    slots: Vec<Slot>,
    reports: Vec<BackendReport>,
    personal: PersonalDictionary,
    prepared: HashMap<(usize, String), Prepared>,
}

impl Engine {
    fn new(factories: Vec<(String, BackendFactory)>, personal: PersonalDictionary) -> Self {
        let mut slots = Vec::new();
        let mut reports = Vec::new();
        for (name, make) in factories {
            match make() {
                Ok(backend) => {
                    reports.push(BackendReport {
                        name: backend.name().to_owned(),
                        running: true,
                        reason: None,
                    });
                    slots.push(Slot { backend });
                }
                Err(reason) => reports.push(BackendReport { name, running: false, reason: Some(reason) }),
            }
        }
        Engine { slots, reports, personal, prepared: HashMap::new() }
    }

    fn prepare(&mut self, backend: usize, language: &str) -> Prepared {
        let key = (backend, language.to_owned());
        if let Some(p) = self.prepared.get(&key) {
            return p.clone();
        }
        let p = match self.slots.get_mut(backend) {
            Some(s) => s.backend.prepare(language),
            None => Prepared::Missing,
        };
        self.prepared.insert(key, p.clone());
        p
    }

    /// The first back end that can check `language`, or why none can.
    fn resolve(&mut self, language: &str) -> Result<(usize, String), Vec<String>> {
        let mut reasons = Vec::new();
        for i in 0..self.slots.len() {
            match self.prepare(i, language) {
                Prepared::Ready { dictionary } => return Ok((i, dictionary)),
                Prepared::Broken(why) => {
                    let name = self.slots.get(i).map_or("?", |s| s.backend.name()).to_owned();
                    reasons.push(format!("{language}: {name}: {why}"));
                }
                Prepared::Missing => {}
            }
        }
        Err(reasons)
    }

    /// The back ends for `languages`; an error when none of them can be checked at all.
    fn usable(&mut self, languages: &[String]) -> Result<Vec<Usable>, SpellError> {
        let mut out = Vec::new();
        let mut reasons = Vec::new();
        for l in languages {
            match self.resolve(l) {
                Ok((backend, _)) => out.push(Usable { language: l.clone(), backend }),
                Err(r) => reasons.extend(r),
            }
        }
        if out.is_empty() {
            if languages.is_empty() {
                reasons.push("no language is chosen".to_owned());
            }
            return Err(SpellError::Unavailable { languages: languages.to_vec(), reasons });
        }
        Ok(out)
    }

    /// Whether `word` is right in any language it is written in. A word no chosen language
    /// is written in is not judged.
    fn word_ok(&mut self, usable: &[Usable], word: &str) -> Result<bool, SpellError> {
        if self.personal.contains(word) {
            return Ok(true);
        }
        let mut judged = false;
        let mut failure = None;
        for u in usable.iter().filter(|u| in_script_of(word, &u.language)) {
            judged = true;
            let Some(slot) = self.slots.get_mut(u.backend) else { continue };
            match slot.backend.check_word(&u.language, word) {
                Ok(true) => return Ok(true),
                Ok(false) => {}
                Err(e) => failure = Some(e),
            }
        }
        match (judged, failure) {
            (false, _) => Ok(true),
            (true, Some(e)) => Err(SpellError::Backend(e)),
            (true, None) => Ok(false),
        }
    }

    fn suggestions(&mut self, languages: &[String], word: &str) -> Result<Vec<String>, SpellError> {
        let usable = self.usable(languages)?;
        let mut lists = Vec::new();
        let mut failure = None;
        for u in usable.iter().filter(|u| in_script_of(word, &u.language)) {
            let Some(slot) = self.slots.get_mut(u.backend) else { continue };
            match slot.backend.suggest(&u.language, word) {
                Ok(l) => lists.push(l),
                Err(e) => failure = Some(e),
            }
        }
        if lists.is_empty()
            && let Some(e) = failure
        {
            return Err(SpellError::Backend(e));
        }
        // Best first from each language in turn, so no language crowds out the others.
        let mut out: Vec<String> = Vec::new();
        let longest = lists.iter().map(Vec::len).max().unwrap_or(0);
        for i in 0..longest {
            for l in &lists {
                if let Some(s) = l.get(i)
                    && s != word
                    && !out.contains(s)
                {
                    out.push(s.clone());
                }
            }
        }
        Ok(out)
    }

    fn begin(
        &mut self,
        text: String,
        languages: &[String],
        cancel: Arc<AtomicBool>,
        reply: CheckReply,
        mut notify: Option<Notify>,
    ) -> Option<Active> {
        match self.usable(languages) {
            Ok(usable) => {
                // Words are cut by the rules of the first language asked for.
                let first = languages.first().map_or("en", String::as_str);
                let words = find_words(&text, first);
                Some(Active { words, next: 0, found: Vec::new(), text, usable, cancel, reply, notify })
            }
            Err(e) => {
                answer(&reply, &mut notify, Err(e));
                None
            }
        }
    }

    /// Checks up to a slice of words. True when the check is over (and has been answered).
    fn step(&mut self, a: &mut Active) -> bool {
        if a.cancel.load(Ordering::Relaxed) {
            return true;
        }
        let end = (a.next + SLICE).min(a.words.len());
        let usable = std::mem::take(&mut a.usable);
        for i in a.next..end {
            let Some(range) = a.words.get(i).cloned() else { continue };
            let Some(word) = a.text.get(range.clone()) else { continue };
            match self.word_ok(&usable, word) {
                Ok(true) => {}
                Ok(false) => a.found.push(range),
                Err(e) => {
                    answer(&a.reply, &mut a.notify, Err(e));
                    return true;
                }
            }
        }
        a.usable = usable;
        a.next = end;
        if a.next >= a.words.len() {
            let found = std::mem::take(&mut a.found);
            answer(&a.reply, &mut a.notify, Ok(found));
            return true;
        }
        false
    }

    fn handle(&mut self, job: Job) {
        match job {
            Job::Check { .. } => {}
            Job::Word { word, languages, reply } => {
                let r = self.usable(&languages).and_then(|u| self.word_ok(&u, &word));
                let _ = reply.send(r);
            }
            Job::Suggest { word, languages, reply } => {
                let _ = reply.send(self.suggestions(&languages, &word));
            }
            Job::Availability { language, reply } => {
                let mut reasons = Vec::new();
                let mut answer = None;
                for i in 0..self.slots.len() {
                    match self.prepare(i, &language) {
                        Prepared::Ready { dictionary } => {
                            let backend =
                                self.slots.get(i).map_or(String::new(), |s| s.backend.name().to_owned());
                            answer = Some(Availability::Ready { backend, dictionary });
                            break;
                        }
                        Prepared::Broken(why) => {
                            let name = self.slots.get(i).map_or("?", |s| s.backend.name()).to_owned();
                            reasons.push(format!("{name}: {why}"));
                        }
                        Prepared::Missing => {}
                    }
                }
                let _ = reply.send(answer.unwrap_or(Availability::Missing { reasons }));
            }
            Job::Report { reply } => {
                let _ = reply.send(self.reports.clone());
            }
            Job::Languages { reply } => {
                let mut out: Vec<String> = Vec::new();
                for s in &mut self.slots {
                    for l in s.backend.languages() {
                        if !out.contains(&l) {
                            out.push(l);
                        }
                    }
                }
                let _ = reply.send(out);
            }
        }
    }
}

/// The checking thread's loop. It ends when every handle to the checker is gone.
pub(crate) fn run(factories: Vec<(String, BackendFactory)>, personal: PersonalDictionary, rx: Receiver<Job>) {
    let mut engine = Engine::new(factories, personal);
    let mut queue: VecDeque<Job> = VecDeque::new();
    let mut active: Option<Active> = None;
    let mut closed = false;
    loop {
        if active.is_none() && queue.is_empty() {
            match rx.recv() {
                Ok(j) => queue.push_back(j),
                Err(_) => break,
            }
        }
        loop {
            match rx.try_recv() {
                Ok(j) => queue.push_back(j),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }
        // Anything but a check goes first.
        if let Some(i) = queue.iter().position(|j| !matches!(j, Job::Check { .. }))
            && let Some(j) = queue.remove(i)
        {
            engine.handle(j);
            continue;
        }
        if active.is_none() {
            // Checks run in the order they were asked for; those whose owner lost interest
            // (the text changed again) are dropped without being started.
            while let Some(j) = queue.pop_front() {
                if let Job::Check { text, languages, cancel, reply, notify } = j
                    && !cancel.load(Ordering::Relaxed)
                {
                    active = engine.begin(text, &languages, cancel, reply, notify);
                    if active.is_some() {
                        break;
                    }
                }
            }
        }
        if let Some(a) = active.as_mut()
            && engine.step(a)
        {
            active = None;
        }
        if closed && active.is_none() && queue.is_empty() {
            break;
        }
    }
}
