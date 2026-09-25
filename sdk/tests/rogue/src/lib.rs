//! A package that misbehaves on request: the host's tests use it to show
//! that a bench running over its budget, trapping, growing past its memory
//! or reaching outside its folder costs the app nothing.

use printcad_bench_sdk::api::*;
use printcad_bench_sdk::{Bench, Value, bench, host, json};

#[derive(Default)]
struct Rogue {
    calls: u32,
}

impl Bench for Rogue {
    fn describe(&self) -> Registration {
        let command = |id: &str| Command {
            id: format!("test.rogue.{id}"),
            summary: id.into(),
            ..Default::default()
        };
        Registration {
            label: "Rogue".into(),
            commands: [
                "count", "spin", "panic", "grow", "files", "job", "helper", "add", "touch",
                "outside",
            ]
            .into_iter()
            .map(command)
            .collect(),
            tools: vec![Tool {
                id: "not.prefixed".into(),
                label: "Left out".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn run_command(&mut self, id: &str, args: Value) -> Result<Value, String> {
        self.calls += 1;
        match id {
            "test.rogue.count" => Ok(json!(self.calls)),
            "test.rogue.spin" => {
                let mut n: u64 = 0;
                loop {
                    n = std::hint::black_box(n.wrapping_add(1));
                }
            }
            "test.rogue.panic" => panic!("on request"),
            "test.rogue.grow" => {
                let mut hoard: Vec<Vec<u8>> = Vec::new();
                loop {
                    hoard.push(vec![1u8; 16 * 1024 * 1024]);
                }
            }
            "test.rogue.files" => {
                std::fs::write("/data/note.txt", b"kept").map_err(|e| e.to_string())?;
                let back = std::fs::read_to_string("/data/note.txt").map_err(|e| e.to_string())?;
                let outside = std::fs::read_to_string("/etc/hostname").is_ok()
                    || std::fs::read_dir("/").is_ok_and(|mut d| d.next().is_some());
                Ok(json!({"back": back, "outside": outside}))
            }
            "test.rogue.job" => {
                let input = args.get("steps").and_then(Value::as_u64).unwrap_or(10);
                host::start_job("count", &input.to_string()).map(|j| json!(j))
            }
            "test.rogue.helper" => host::start_job("helper", "hello").map(|j| json!(j)),
            "test.rogue.add" => {
                host::add_feature("test.rogue.thing", "Thing", None, json!({})).map(|id| json!(id))
            }
            "test.rogue.touch" => {
                let id = args.get("id").and_then(Value::as_str).ok_or("id")?;
                host::set_feature_data(id, json!({"touched": true})).map(|_| Value::Null)
            }
            "test.rogue.outside" => {
                host::add_feature("test.other.kind", "Other", None, json!({})).map(|id| json!(id))
            }
            _ => Err(format!("no command `{id}`")),
        }
    }

    fn input(&mut self, input: &Input) -> bool {
        if let Event::JobFinished { job, result } = &input.event {
            host::info(&format!("job {job}: {result:?}"));
            return true;
        }
        false
    }

    fn job(entry: &str, input: &str) -> Result<String, String> {
        if entry == "helper" {
            let out = host::helper("upper", input.as_bytes())?;
            return String::from_utf8(out).map_err(|e| e.to_string());
        }
        if entry != "count" {
            return Err(format!("no job `{entry}`"));
        }
        let steps: u64 = input.parse().map_err(|_| "steps")?;
        let mut sum: u64 = 0;
        for i in 0..steps {
            if host::cancelled() {
                return Err("stopped".into());
            }
            for k in 0..1000u64 {
                sum = std::hint::black_box(sum.wrapping_add(i * k));
            }
            host::progress(i + 1, steps);
        }
        Ok(sum.to_string())
    }
}

bench!(Rogue);
