//! Exercises the production connector without generating or publishing answers.
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: first operation, FD9 is reserved by the documented probe launcher.
    let credential = unsafe { dl_ai::take_knowledge_credential() };
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(credential))
}
async fn run(
    credential: Option<dl_ai::InheritedKnowledgeCredential>,
) -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 2 {
        return Err("usage: knowledge_router_probe CONFIG CASES_JSON".into());
    }
    let router = dl_ai::load_knowledge_router(Path::new(&args[0]), credential)
        .await?
        .ok_or("router disabled")?;
    let cases: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    for case in cases {
        let request: dl_ai::KnowledgeRouteRequest =
            serde_json::from_value(case["request"].clone())?;
        let started = std::time::Instant::now();
        let decision = router.route(request).await;
        let output = match decision {
            Ok(decision) => {
                serde_json::json!({"id":case["id"],"elapsed_ms":started.elapsed().as_millis(),"decision":decision})
            }
            Err(error) => {
                serde_json::json!({"id":case["id"],"elapsed_ms":started.elapsed().as_millis(),"error":error.to_string()})
            }
        };
        println!("{output}");
    }
    Ok(())
}
