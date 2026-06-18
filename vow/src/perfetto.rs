//! Perfetto / Chrome "Trace Event Format" tracing for a single compilation.
//!
//! Emits a gzipped JSON trace loadable at ui.perfetto.dev. The trace is a pure
//! side artifact: it never feeds codegen, the build JSON, or any cache key.

/// Perfetto pid/tid layout for one compilation. The compiler is a single
/// "process"; codegen/frontend share the main thread, verification has its own
/// driver thread, and each verify worker gets a distinct thread track.
pub const PID_COMPILER: u64 = 1;
pub const TID_MAIN: u64 = 1;
pub const TID_VERIFY_DRIVER: u64 = 2;
pub const TID_WORKER_BASE: u64 = 100;

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// One trace event in a neutral form, later mapped to a Chrome Trace Event.
#[derive(Clone, Debug)]
pub struct RawEvent {
    pub name: String,
    pub pid: u64,
    pub tid: u64,
    pub ts_us: u64,
    pub kind: EventKind,
    pub args: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub enum EventKind {
    /// Complete duration event (`ph: "X"`).
    Span { dur_us: u64 },
    /// Counter sample (`ph: "C"`): one or more named numeric series.
    Counter { series: Vec<(String, f64)> },
    /// Flow event endpoint (`ph: "s"` start / `ph: "f"` finish), drawing an
    /// arrow between matching `id`s.
    Flow { id: u64, edge: FlowEdge },
}

#[derive(Clone, Copy, Debug)]
pub enum FlowEdge {
    Start,
    End,
}

/// Thread-safe collector handle. `Clone` shares the same underlying buffer
/// (Arc), so clones may be moved into worker threads.
#[derive(Clone)]
pub struct Profiler {
    t0: Instant,
    events: Arc<Mutex<Vec<RawEvent>>>,
}

impl Default for Profiler {
    fn default() -> Self {
        Self::new()
    }
}

impl Profiler {
    pub fn new() -> Self {
        Profiler {
            t0: Instant::now(),
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Microseconds since this profiler's clock origin.
    pub fn now_us(&self) -> u64 {
        self.t0.elapsed().as_micros() as u64
    }

    fn push(&self, ev: RawEvent) {
        // A poisoned lock only means another thread panicked mid-push; recover
        // the buffer rather than cascading the panic into the compile pipeline.
        let mut guard = match self.events.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.push(ev);
    }

    pub fn span(
        &self,
        name: &str,
        pid: u64,
        tid: u64,
        start_us: u64,
        dur_us: u64,
        args: Vec<(String, String)>,
    ) {
        self.push(RawEvent {
            name: name.to_string(),
            pid,
            tid,
            ts_us: start_us,
            kind: EventKind::Span { dur_us },
            args,
        });
    }

    /// Record a counter sample. `series` are `(name, value)` pairs plotted as
    /// numeric tracks under the counter group `name` for process `pid`.
    pub fn counter(&self, name: &str, pid: u64, ts_us: u64, series: Vec<(String, f64)>) {
        self.push(RawEvent {
            name: name.to_string(),
            pid,
            tid: 0,
            ts_us,
            kind: EventKind::Counter { series },
            args: Vec::new(),
        });
    }

    /// Record one endpoint of a flow arrow. Matching `id`s on a `Start` and an
    /// `End` draw an arrow (e.g. the compiler→ESBMC handoff).
    pub fn flow(&self, name: &str, pid: u64, tid: u64, ts_us: u64, id: u64, edge: FlowEdge) {
        self.push(RawEvent {
            name: name.to_string(),
            pid,
            tid,
            ts_us,
            kind: EventKind::Flow { id, edge },
            args: Vec::new(),
        });
    }

    /// A clone of all events recorded so far, for serialization.
    pub fn snapshot(&self) -> Vec<RawEvent> {
        match self.events.lock() {
            Ok(g) => g.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Spawn a background thread that samples this process and its ESBMC
    /// children every `interval`, emitting counter events. The returned handle
    /// stops the thread on `stop()` (or on drop).
    pub fn start_sampler(&self, interval: Duration) -> ResourceSampler {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let prof = self.clone();
        let handle = std::thread::spawn(move || {
            let own_pid = match sysinfo::get_current_pid() {
                Ok(p) => p.as_u32() as u64,
                Err(_) => return,
            };
            let mut sys = sysinfo::System::new();
            while !stop_thread.load(Ordering::Relaxed) {
                // sysinfo CPU% is a delta between refreshes; a process first
                // seen this tick reports 0% (RSS is correct immediately).
                let refresh = sysinfo::ProcessRefreshKind::nothing()
                    .with_memory()
                    .with_cpu();
                sys.refresh_processes_specifics(sysinfo::ProcessesToUpdate::All, true, refresh);
                let procs: Vec<ProcInfo> = sys
                    .processes()
                    .values()
                    .map(|p| ProcInfo {
                        pid: p.pid().as_u32() as u64,
                        parent: p.parent().map(|pp| pp.as_u32() as u64),
                        name: p.name().to_string_lossy().into_owned(),
                        rss_kb: (p.memory() / 1024) as f64,
                        cpu_pct: p.cpu_usage() as f64,
                    })
                    .collect();
                let ts = prof.now_us();
                for s in collect_samples(&procs, own_pid) {
                    // Place the compiler's counters on PID_COMPILER so they
                    // share the process row with the compiler's spans; ESBMC
                    // children keep their real pid as their own process track.
                    let track_pid = if s.group == "compiler" {
                        PID_COMPILER
                    } else {
                        s.pid
                    };
                    prof.counter(
                        &s.group,
                        track_pid,
                        ts,
                        vec![
                            ("rss_kb".to_string(), s.rss_kb),
                            ("cpu_pct".to_string(), s.cpu_pct),
                        ],
                    );
                }
                std::thread::sleep(interval);
            }
        });
        ResourceSampler {
            stop,
            handle: Some(handle),
        }
    }
}

/// Handle to a running [`Profiler`] resource sampler. Stops the sampling thread
/// when `stop()` is called or when dropped.
pub struct ResourceSampler {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl ResourceSampler {
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for ResourceSampler {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// A snapshot of one OS process, decoupled from `sysinfo` so the sampling
/// logic is unit-testable with synthetic process tables.
#[derive(Clone, Debug)]
pub struct ProcInfo {
    pub pid: u64,
    pub parent: Option<u64>,
    pub name: String,
    pub rss_kb: f64,
    pub cpu_pct: f64,
}

/// One resource sample to emit as a counter group.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub group: String,
    pub pid: u64,
    pub rss_kb: f64,
    pub cpu_pct: f64,
}

/// Reduce a process table to the samples we trace: the compiler process itself
/// (`group = "compiler"`), plus one group per ESBMC child of ours
/// (`group = "esbmc:<pid>"`) summing that child's whole descendant subtree so
/// the SMT solver's memory (z3/boolector run as ESBMC's children) is counted.
/// Excludes unrelated system `esbmc` processes (parent check) and the linker
/// child (name check).
pub fn collect_samples(procs: &[ProcInfo], own_pid: u64) -> Vec<Sample> {
    let mut samples = Vec::new();
    if let Some(me) = procs.iter().find(|p| p.pid == own_pid) {
        samples.push(Sample {
            group: "compiler".to_string(),
            pid: own_pid,
            rss_kb: me.rss_kb,
            cpu_pct: me.cpu_pct,
        });
    }

    let mut children: HashMap<u64, Vec<u64>> = HashMap::new();
    for p in procs {
        if let Some(par) = p.parent {
            children.entry(par).or_default().push(p.pid);
        }
    }
    let by_pid: HashMap<u64, &ProcInfo> = procs.iter().map(|p| (p.pid, p)).collect();

    // Direct ESBMC children of the compiler, in deterministic pid order.
    let mut esbmc_children: Vec<&ProcInfo> = procs
        .iter()
        .filter(|p| p.parent == Some(own_pid) && p.name.starts_with("esbmc"))
        .collect();
    esbmc_children.sort_by_key(|p| p.pid);

    for child in esbmc_children {
        // Sum the child's whole descendant subtree (esbmc + its solver procs).
        let (mut rss_kb, mut cpu_pct) = (0.0, 0.0);
        let mut stack = vec![child.pid];
        while let Some(pid) = stack.pop() {
            if let Some(p) = by_pid.get(&pid) {
                rss_kb += p.rss_kb;
                cpu_pct += p.cpu_pct;
            }
            if let Some(kids) = children.get(&pid) {
                stack.extend(kids.iter().copied());
            }
        }
        samples.push(Sample {
            group: format!("esbmc:{}", child.pid),
            pid: child.pid,
            rss_kb,
            cpu_pct,
        });
    }

    samples
}

fn args_object(args: &[(String, String)]) -> serde_json::Value {
    let map: serde_json::Map<String, serde_json::Value> = args
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect();
    serde_json::Value::Object(map)
}

/// Serialize events to the Chrome Trace Event Format and write them gzipped to
/// `path`. ui.perfetto.dev auto-decompresses a single gzip stream.
pub fn write_trace_gz(events: &[RawEvent], path: &Path) -> io::Result<()> {
    let mut trace_events = Vec::with_capacity(events.len());
    for e in events {
        match &e.kind {
            EventKind::Span { dur_us } => {
                trace_events.push(serde_json::json!({
                    "ph": "X",
                    "name": e.name,
                    "pid": e.pid,
                    "tid": e.tid,
                    "ts": e.ts_us,
                    "dur": dur_us,
                    "args": args_object(&e.args),
                }));
            }
            EventKind::Counter { series } => {
                let map: serde_json::Map<String, serde_json::Value> = series
                    .iter()
                    .filter_map(|(k, v)| {
                        serde_json::Number::from_f64(*v)
                            .map(|n| (k.clone(), serde_json::Value::Number(n)))
                    })
                    .collect();
                trace_events.push(serde_json::json!({
                    "ph": "C",
                    "name": e.name,
                    "pid": e.pid,
                    "ts": e.ts_us,
                    "args": serde_json::Value::Object(map),
                }));
            }
            EventKind::Flow { id, edge } => {
                let ph = match edge {
                    FlowEdge::Start => "s",
                    FlowEdge::End => "f",
                };
                let mut ev = serde_json::json!({
                    "ph": ph,
                    "id": id,
                    "name": e.name,
                    "cat": e.name,
                    "pid": e.pid,
                    "tid": e.tid,
                    "ts": e.ts_us,
                });
                // Bind the finish endpoint to the enclosing slice so the arrow
                // lands on the target span rather than the next one.
                if matches!(edge, FlowEdge::End) {
                    ev["bp"] = serde_json::Value::String("e".to_string());
                }
                trace_events.push(ev);
            }
        }
    }
    let doc = serde_json::json!({
        "traceEvents": trace_events,
        "displayTimeUnit": "ms",
    });
    let bytes = serde_json::to_vec(&doc)?;

    let file = std::fs::File::create(path)?;
    let mut enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    enc.write_all(&bytes)?;
    enc.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn decode(path: &std::path::Path) -> serde_json::Value {
        let f = std::fs::File::open(path).unwrap();
        let mut gz = flate2::read::GzDecoder::new(f);
        let mut s = String::new();
        gz.read_to_string(&mut s).unwrap();
        serde_json::from_str(&s).unwrap()
    }

    #[test]
    fn span_serializes_as_complete_event() {
        let prof = Profiler::new();
        prof.span("parse", 1, 1, 0, 1200, vec![]);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json.gz");
        write_trace_gz(&prof.snapshot(), &path).unwrap();

        let v = decode(&path);
        let events = v["traceEvents"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e["ph"], "X");
        assert_eq!(e["name"], "parse");
        assert_eq!(e["ts"], 0);
        assert_eq!(e["dur"], 1200);
        assert_eq!(e["pid"], 1);
        assert_eq!(e["tid"], 1);
    }

    #[test]
    fn counter_serializes_with_numeric_series() {
        let prof = Profiler::new();
        prof.counter("compiler", 1, 50, vec![("rss_kb".into(), 4096.0)]);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json.gz");
        write_trace_gz(&prof.snapshot(), &path).unwrap();

        let v = decode(&path);
        let e = &v["traceEvents"].as_array().unwrap()[0];
        assert_eq!(e["ph"], "C");
        assert_eq!(e["name"], "compiler");
        assert_eq!(e["pid"], 1);
        assert_eq!(e["ts"], 50);
        // Counter series must be a JSON number so Perfetto can plot it.
        assert_eq!(e["args"]["rss_kb"], 4096.0);
    }

    #[test]
    fn flow_serializes_start_and_end_with_matching_id() {
        let prof = Profiler::new();
        prof.flow("handoff", 1, 2, 100, 7, FlowEdge::Start);
        prof.flow("handoff", 1, 3, 120, 7, FlowEdge::End);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json.gz");
        write_trace_gz(&prof.snapshot(), &path).unwrap();

        let v = decode(&path);
        let evs = v["traceEvents"].as_array().unwrap();
        assert_eq!(evs[0]["ph"], "s");
        assert_eq!(evs[0]["id"], 7);
        assert_eq!(evs[0]["name"], "handoff");
        assert_eq!(evs[1]["ph"], "f");
        assert_eq!(evs[1]["id"], 7);
    }

    #[test]
    fn records_concurrently_without_loss() {
        let prof = Profiler::new();
        let mut handles = Vec::new();
        for t in 0..8u64 {
            let p = prof.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..100u64 {
                    p.span("work", 1, t, i, 1, vec![]);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(prof.snapshot().len(), 800);
    }

    #[test]
    fn collect_samples_reports_compiler_self() {
        let procs = vec![ProcInfo {
            pid: 1,
            parent: None,
            name: "vow".into(),
            rss_kb: 1000.0,
            cpu_pct: 10.0,
        }];
        let samples = collect_samples(&procs, 1);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].group, "compiler");
        assert_eq!(samples[0].pid, 1);
        assert_eq!(samples[0].rss_kb, 1000.0);
        assert_eq!(samples[0].cpu_pct, 10.0);
    }

    #[test]
    fn collect_samples_sums_esbmc_subtree_and_filters() {
        let procs = vec![
            ProcInfo {
                pid: 1,
                parent: None,
                name: "vow".into(),
                rss_kb: 1000.0,
                cpu_pct: 5.0,
            },
            // ESBMC child of ours.
            ProcInfo {
                pid: 2,
                parent: Some(1),
                name: "esbmc".into(),
                rss_kb: 2000.0,
                cpu_pct: 20.0,
            },
            // Solver grandchild — must fold into the esbmc:2 group.
            ProcInfo {
                pid: 3,
                parent: Some(2),
                name: "z3".into(),
                rss_kb: 5000.0,
                cpu_pct: 50.0,
            },
            // Linker child of ours — excluded by name.
            ProcInfo {
                pid: 4,
                parent: Some(1),
                name: "cc".into(),
                rss_kb: 800.0,
                cpu_pct: 1.0,
            },
            // Unrelated system esbmc (different parent) — excluded.
            ProcInfo {
                pid: 5,
                parent: Some(99),
                name: "esbmc".into(),
                rss_kb: 9999.0,
                cpu_pct: 9.0,
            },
        ];
        let samples = collect_samples(&procs, 1);

        let esbmc: Vec<&Sample> = samples
            .iter()
            .filter(|s| s.group.starts_with("esbmc"))
            .collect();
        assert_eq!(esbmc.len(), 1, "exactly one esbmc group");
        assert_eq!(esbmc[0].pid, 2);
        assert_eq!(esbmc[0].rss_kb, 7000.0, "esbmc + z3 subtree summed");
        assert_eq!(esbmc[0].cpu_pct, 70.0);
        // Linker (pid 4) and unrelated esbmc (pid 5) never appear as groups.
        assert!(samples.iter().all(|s| s.pid != 4 && s.pid != 5));
    }

    #[test]
    fn sampler_emits_self_memory_counter() {
        use std::time::Duration;
        let prof = Profiler::new();
        let sampler = prof.start_sampler(Duration::from_millis(5));
        std::thread::sleep(Duration::from_millis(60));
        sampler.stop();

        let snap = prof.snapshot();
        let has_compiler = snap
            .iter()
            .any(|e| e.name == "compiler" && matches!(e.kind, EventKind::Counter { .. }));
        assert!(
            has_compiler,
            "expected at least one compiler counter sample"
        );
    }
}
