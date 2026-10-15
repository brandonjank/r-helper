//! Command dispatch runs before GUI ownership or hardware monitoring starts.

type Handler = fn() -> Option<anyhow::Result<()>>;

pub(crate) fn dispatch() -> bool {
    let handlers: &[(&str, Handler)] = &[
        ("Updates", crate::blade_updates::dispatch),
        ("GPU preview", crate::blade_gpu::dispatch),
        ("Platform diagnostics", crate::blade_platform::dispatch),
        ("Auxiliary lighting", crate::blade_auxiliary::dispatch),
        ("Razer ownership", crate::blade_vendor::dispatch),
        ("Batch test", crate::blade_batch_test::dispatch),
        ("Display recovery", crate::blade_display::dispatch),
    ];
    if let Some((label, result)) = first_match(handlers) {
        if let Err(error) = result {
            eprintln!("{label}: {error:#}");
            std::process::exit(1);
        }
        return true;
    }
    false
}

fn first_match<'a>(handlers: &[(&'a str, Handler)]) -> Option<(&'a str, anyhow::Result<()>)> {
    handlers.iter().find_map(|(label, handler)| handler().map(|result| (*label, result)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unrelated() -> Option<anyhow::Result<()>> {
        None
    }
    fn handled() -> Option<anyhow::Result<()>> {
        Some(Ok(()))
    }
    fn failed() -> Option<anyhow::Result<()>> {
        Some(Err(anyhow::anyhow!("recovery failed")))
    }
    fn must_not_run() -> Option<anyhow::Result<()>> {
        panic!("later handler ran")
    }

    #[test]
    fn dispatch_stops_after_success_or_failure_before_later_side_effects() {
        for handler in [handled as Handler, failed as Handler] {
            let result =
                first_match(&[("skip", unrelated), ("recovery", handler), ("GUI", must_not_run)])
                    .unwrap();
            assert_eq!(result.0, "recovery");
            assert_eq!(result.1.is_ok(), handler().unwrap().is_ok());
        }
        assert!(first_match(&[("unrelated", unrelated)]).is_none());
    }
}
