use sketch_solver::replay::{Fixture, Regression, run};
use std::io::Read;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let filename = arguments
        .next()
        .ok_or("usage: solve_problem <fixture.json|corpus.json|-> [revision]")?;
    let revision = arguments.next();
    if arguments.next().is_some() {
        return Err("too many arguments".into());
    }
    let text = if filename == "-" {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        text
    } else {
        std::fs::read_to_string(&filename)?
    };
    let value: serde_json::Value = serde_json::from_str(&text)?;
    let result = if value.is_array() {
        let corpus: Vec<Regression> = serde_json::from_value(value)?;
        serde_json::Value::Array(
            corpus
                .iter()
                .map(|case| serde_json::json!({ "name": case.name, "result": run(&case.fixture) }))
                .collect(),
        )
    } else {
        let fixture: Fixture = serde_json::from_value(value)?;
        serde_json::to_value(run(&fixture))?
    };
    let output = serde_json::json!({ "schema_version": sketch_solver::replay::SCHEMA_VERSION, "revision": revision, "result": result });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
