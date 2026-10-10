use std::ffi::OsString;
use std::process::ExitCode;

use prism::single_instance::{self, Claim};
use prism::{App, Environment};

fn main() -> ExitCode {
    let mut environment = match environment() {
        Ok(environment) => environment,
        Err(message) => {
            eprintln!("prism: {message}");
            return ExitCode::from(2);
        }
    };
    match single_instance::claim(single_instance::launch_activation_token()) {
        Ok(Claim::First(launches)) => environment.host.launches = Box::new(launches),
        Ok(Claim::HandedOver) => {
            eprintln!("prism: Prism is already running, so its window was raised instead");
            return ExitCode::SUCCESS;
        }
        // Prism still runs, but launching it again opens another window.
        Err(error) => eprintln!("prism: can't reach the session bus to run only once: {error}"),
    }
    match App::new(environment).and_then(App::run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("prism: {error}");
            ExitCode::FAILURE
        }
    }
}

fn environment() -> Result<Environment, String> {
    let mut environment = Environment::production();
    if let Some(fixture) = std::env::var_os("PRISM_SIMULATE") {
        simulate(&mut environment, fixture)?;
    }
    Ok(environment)
}

/// Swaps the GPUs for a simulated fixture, played against Prism's own clock.
#[cfg(feature = "simulated")]
fn simulate(environment: &mut Environment, fixture: OsString) -> Result<(), String> {
    use std::sync::Arc;

    use prism_gpu::GpuProviders;
    use prism_gpu::simulated::{SimulatedFixture, SimulatedGpus};

    let fixture: SimulatedFixture = fixture
        .to_str()
        .unwrap_or_default()
        .parse()
        .map_err(|error| format!("PRISM_SIMULATE: {error}"))?;
    let clock = environment.clock.clone();
    let gpus = Arc::new(SimulatedGpus::new(fixture, move || clock.now()));
    environment.gpus = GpuProviders::empty()
        .with_inventory(gpus.clone())
        .with_telemetry(gpus);
    Ok(())
}

#[cfg(not(feature = "simulated"))]
fn simulate(_: &mut Environment, _: OsString) -> Result<(), String> {
    Err("PRISM_SIMULATE needs a build with the `simulated` feature".into())
}
