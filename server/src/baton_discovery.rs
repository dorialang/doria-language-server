use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use crate::project::ProjectDocument;

const PROJECT_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const MAX_STDOUT: usize = 8 * 1024 * 1024;
const MAX_STDERR: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BatonCommandSpec {
    pub(crate) program: PathBuf,
    pub(crate) arguments: Vec<String>,
    pub(crate) current_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct DiscoveryRequest {
    pub(crate) root_uri: String,
    pub(crate) root_path: PathBuf,
    pub(crate) baton_override: Option<String>,
}

#[derive(Debug)]
pub(crate) struct DiscoveryUpdate {
    pub(crate) root_uri: String,
    pub(crate) result: Result<ProjectDocument, String>,
}

trait DiscoveryRunner: Send + Sync {
    fn discover(
        &self,
        request: &DiscoveryRequest,
        cancelled: &AtomicBool,
    ) -> Result<ProjectDocument, String>;
}

#[derive(Default)]
struct ProcessDiscoveryRunner;

impl DiscoveryRunner for ProcessDiscoveryRunner {
    fn discover(
        &self,
        request: &DiscoveryRequest,
        cancelled: &AtomicBool,
    ) -> Result<ProjectDocument, String> {
        let baton = resolve_baton(
            request.baton_override.as_deref(),
            env::var_os("DORIA_BATON_PATH").as_deref(),
            env::current_exe().ok().as_deref(),
            env::var_os("PATH").as_deref(),
        )?;
        let workspace = project_command(&baton, &request.root_path, true);
        match run_project_command(&workspace, cancelled) {
            Ok(output) => ProjectDocument::parse(&output),
            Err(error) if error.contains("Workspace Package Selection Is Unavailable") => {
                let package = project_command(&baton, &request.root_path, false);
                ProjectDocument::parse(&run_project_command(&package, cancelled)?)
            }
            Err(error) => Err(error),
        }
    }
}

struct PendingDiscovery {
    generation: u64,
    due: Instant,
    request: DiscoveryRequest,
}

struct ActiveDiscovery {
    cancelled: Arc<AtomicBool>,
}

struct ThreadResult {
    generation: u64,
    update: DiscoveryUpdate,
}

pub(crate) struct ProjectDiscovery {
    runner: Arc<dyn DiscoveryRunner>,
    sender: mpsc::Sender<ThreadResult>,
    receiver: mpsc::Receiver<ThreadResult>,
    pending: HashMap<String, PendingDiscovery>,
    active: HashMap<String, ActiveDiscovery>,
    generations: HashMap<String, u64>,
}

impl Default for ProjectDiscovery {
    fn default() -> Self {
        Self::with_runner(Arc::new(ProcessDiscoveryRunner))
    }
}

impl ProjectDiscovery {
    fn with_runner(runner: Arc<dyn DiscoveryRunner>) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            runner,
            sender,
            receiver,
            pending: HashMap::new(),
            active: HashMap::new(),
            generations: HashMap::new(),
        }
    }

    pub(crate) fn schedule(&mut self, request: DiscoveryRequest, delay: Duration) {
        let generation = self.advance_generation(&request.root_uri);
        if let Some(active) = self.active.remove(&request.root_uri) {
            active.cancelled.store(true, Ordering::Release);
        }
        self.pending.insert(
            request.root_uri.clone(),
            PendingDiscovery {
                generation,
                due: Instant::now() + delay,
                request,
            },
        );
    }

    pub(crate) fn cancel(&mut self, root_uri: &str) {
        self.pending.remove(root_uri);
        if let Some(active) = self.active.remove(root_uri) {
            active.cancelled.store(true, Ordering::Release);
        }
        self.advance_generation(root_uri);
    }

    pub(crate) fn poll(&mut self) -> Vec<DiscoveryUpdate> {
        self.start_due();
        let mut updates = Vec::new();
        while let Ok(result) = self.receiver.try_recv() {
            let current = self.generations.get(&result.update.root_uri).copied();
            if current != Some(result.generation) {
                continue;
            }
            self.active.remove(&result.update.root_uri);
            updates.push(result.update);
        }
        updates
    }

    fn start_due(&mut self) {
        let now = Instant::now();
        let roots = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.due <= now)
            .map(|(root, _)| root.clone())
            .collect::<Vec<_>>();
        for root in roots {
            let Some(pending) = self.pending.remove(&root) else {
                continue;
            };
            let cancelled = Arc::new(AtomicBool::new(false));
            self.active.insert(
                root,
                ActiveDiscovery {
                    cancelled: Arc::clone(&cancelled),
                },
            );
            let sender = self.sender.clone();
            let runner = Arc::clone(&self.runner);
            thread::spawn(move || {
                let result = runner.discover(&pending.request, &cancelled);
                let _ = sender.send(ThreadResult {
                    generation: pending.generation,
                    update: DiscoveryUpdate {
                        root_uri: pending.request.root_uri,
                        result,
                    },
                });
            });
        }
    }

    fn advance_generation(&mut self, root_uri: &str) -> u64 {
        let generation = self.generations.entry(root_uri.to_string()).or_default();
        *generation = generation
            .checked_add(1)
            .expect("project discovery generation overflow");
        *generation
    }

    #[cfg(test)]
    pub(crate) fn pending_generation(&self, root_uri: &str) -> Option<u64> {
        self.pending.get(root_uri).map(|pending| pending.generation)
    }
}

pub(crate) fn project_command(baton: &Path, root: &Path, workspace: bool) -> BatonCommandSpec {
    let mut arguments = vec!["project", "--json"];
    if workspace {
        arguments.push("--workspace");
    }
    arguments.extend(["--development", "--offline"]);
    BatonCommandSpec {
        program: baton.to_path_buf(),
        arguments: arguments.into_iter().map(String::from).collect(),
        current_dir: root.to_path_buf(),
    }
}

fn run_project_command(
    command: &BatonCommandSpec,
    cancelled: &AtomicBool,
) -> Result<String, String> {
    if cancelled.load(Ordering::Acquire) {
        return Err("Baton project discovery was superseded".to_string());
    }
    let mut child = Command::new(&command.program)
        .args(&command.arguments)
        .current_dir(&command.current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start Baton project discovery: {error}"))?;
    let stdout = read_bounded(child.stdout.take(), MAX_STDOUT);
    let stderr = read_bounded(child.stderr.take(), MAX_STDERR);
    let started = Instant::now();
    let status = loop {
        if cancelled.load(Ordering::Acquire) {
            stop_child(&mut child);
            return Err("Baton project discovery was superseded".to_string());
        }
        if started.elapsed() >= PROJECT_TIMEOUT {
            stop_child(&mut child);
            return Err("Baton project discovery exceeded 30 seconds".to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(error) => {
                stop_child(&mut child);
                return Err(format!(
                    "could not wait for Baton project discovery: {error}"
                ));
            }
        }
    };
    let stdout = stdout
        .join()
        .map_err(|_| "Baton stdout reader failed".to_string())??;
    let stderr = stderr
        .join()
        .map_err(|_| "Baton stderr reader failed".to_string())??;
    if !status.success() {
        return Err(format!(
            "Baton project discovery failed: {}",
            bounded_message(&stderr)
        ));
    }
    String::from_utf8(stdout).map_err(|_| "Baton project JSON is not valid UTF-8".to_string())
}

fn read_bounded(
    stream: Option<impl Read + Send + 'static>,
    limit: usize,
) -> thread::JoinHandle<Result<Vec<u8>, String>> {
    thread::spawn(move || {
        let Some(stream) = stream else {
            return Ok(Vec::new());
        };
        let mut bytes = Vec::new();
        stream
            .take((limit + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("could not read Baton output: {error}"))?;
        if bytes.len() > limit {
            return Err(format!("Baton output exceeded {limit} bytes"));
        }
        Ok(bytes)
    })
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn bounded_message(stderr: &[u8]) -> String {
    let message = String::from_utf8_lossy(stderr);
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        "the command exited without diagnostics".to_string()
    } else {
        normalized
    }
}

fn resolve_baton(
    explicit: Option<&str>,
    environment: Option<&std::ffi::OsStr>,
    current_executable: Option<&Path>,
    search_path: Option<&std::ffi::OsStr>,
) -> Result<PathBuf, String> {
    if let Some(path) = explicit.filter(|path| !path.trim().is_empty()) {
        return resolve_requested_path(path, "configured Baton path");
    }
    if let Some(path) = environment.and_then(std::ffi::OsStr::to_str) {
        if !path.trim().is_empty() {
            return resolve_requested_path(path, "DORIA_BATON_PATH");
        }
    }
    if let Some(directory) = current_executable.and_then(Path::parent) {
        for name in executable_names("baton") {
            let candidate = directory.join(name);
            if is_executable(&candidate) {
                return Ok(candidate);
            }
        }
    }
    if let Some(path) = search_path {
        for directory in env::split_paths(path) {
            for name in executable_names("baton") {
                let candidate = directory.join(name);
                if is_executable(&candidate) {
                    return Ok(candidate);
                }
            }
        }
    }
    Err("Baton was not found; configure a Baton path, set DORIA_BATON_PATH, install it beside doria-lsp, or add baton to PATH".to_string())
}

fn resolve_requested_path(value: &str, label: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value.trim());
    let path = if path.is_absolute() {
        path
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    if is_executable(&path) {
        Ok(path)
    } else {
        Err(format!(
            "{label} is not an executable file: {}",
            path.display()
        ))
    }
}

fn executable_names(name: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![format!("{name}.exe"), name.to_string()]
    } else {
        vec![name.to_string()]
    }
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    struct FakeRunner {
        calls: Arc<AtomicUsize>,
        delay: Duration,
    }

    impl DiscoveryRunner for FakeRunner {
        fn discover(
            &self,
            _request: &DiscoveryRequest,
            cancelled: &AtomicBool,
        ) -> Result<ProjectDocument, String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let started = Instant::now();
            while started.elapsed() < self.delay {
                if cancelled.load(Ordering::Acquire) {
                    return Err("cancelled".to_string());
                }
                thread::sleep(Duration::from_millis(1));
            }
            Err("fake project result".to_string())
        }
    }

    #[test]
    fn project_arguments_are_offline_and_never_shell_constructed() {
        let command = project_command(Path::new("/toolchain/baton"), Path::new("/workspace"), true);
        assert_eq!(command.program, Path::new("/toolchain/baton"));
        assert_eq!(
            command.arguments,
            [
                "project",
                "--json",
                "--workspace",
                "--development",
                "--offline"
            ]
        );
        assert_eq!(command.current_dir, Path::new("/workspace"));
    }

    #[test]
    fn override_environment_sibling_and_path_resolution_are_ordered() {
        let root = env::temp_dir().join(format!("doria-baton-resolution-{}", std::process::id()));
        let explicit = root.join("explicit/baton");
        let environment = root.join("environment/baton");
        let sibling = root.join("toolchain/baton");
        let path = root.join("path/baton");
        for candidate in [&explicit, &environment, &sibling, &path] {
            fs::create_dir_all(candidate.parent().unwrap()).unwrap();
            fs::write(candidate, b"baton").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(candidate, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let executable = sibling.parent().unwrap().join("doria-lsp");
        assert_eq!(
            resolve_baton(
                explicit.to_str(),
                Some(environment.as_os_str()),
                Some(&executable),
                Some(path.parent().unwrap().as_os_str()),
            )
            .unwrap(),
            explicit
        );
        assert_eq!(
            resolve_baton(
                None,
                Some(environment.as_os_str()),
                Some(&executable),
                Some(path.parent().unwrap().as_os_str()),
            )
            .unwrap(),
            environment
        );
        assert_eq!(
            resolve_baton(
                None,
                None,
                Some(&executable),
                Some(path.parent().unwrap().as_os_str())
            )
            .unwrap(),
            sibling
        );
        assert_eq!(
            resolve_baton(None, None, None, Some(path.parent().unwrap().as_os_str())).unwrap(),
            path
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scheduling_is_non_blocking_debounced_and_cancels_superseded_work() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut discovery = ProjectDiscovery::with_runner(Arc::new(FakeRunner {
            calls: Arc::clone(&calls),
            delay: Duration::from_millis(25),
        }));
        let request = DiscoveryRequest {
            root_uri: "file:///workspace".to_string(),
            root_path: PathBuf::from("/workspace"),
            baton_override: None,
        };
        discovery.schedule(request.clone(), Duration::from_millis(20));
        discovery.schedule(request.clone(), Duration::from_millis(20));
        assert!(discovery.poll().is_empty());
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        thread::sleep(Duration::from_millis(25));
        assert!(discovery.poll().is_empty());
        let deadline = Instant::now() + Duration::from_millis(100);
        while calls.load(Ordering::Relaxed) == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        discovery.schedule(request, Duration::ZERO);
        discovery.poll();
        thread::sleep(Duration::from_millis(35));
        let updates = discovery.poll();
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert_eq!(updates.len(), 1);
        assert_eq!(
            updates[0].result.as_ref().unwrap_err(),
            "fake project result"
        );
    }

    #[test]
    fn cancelled_roots_keep_monotonic_generations_and_reject_stale_results() {
        let mut discovery = ProjectDiscovery::with_runner(Arc::new(FakeRunner {
            calls: Arc::new(AtomicUsize::new(0)),
            delay: Duration::ZERO,
        }));
        let request = DiscoveryRequest {
            root_uri: "file:///workspace".to_string(),
            root_path: PathBuf::from("/workspace"),
            baton_override: None,
        };

        discovery.schedule(request.clone(), Duration::from_secs(60));
        assert_eq!(discovery.pending_generation(&request.root_uri), Some(1));
        discovery.cancel(&request.root_uri);
        discovery.schedule(request.clone(), Duration::from_secs(60));
        assert_eq!(discovery.pending_generation(&request.root_uri), Some(3));

        discovery
            .sender
            .send(ThreadResult {
                generation: 1,
                update: DiscoveryUpdate {
                    root_uri: request.root_uri,
                    result: Err("stale project result".to_string()),
                },
            })
            .unwrap();
        assert!(discovery.poll().is_empty());
    }
}
