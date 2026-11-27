//! The machines page.

use emberhut_api::machines::{MachineState, MachinesPage, WarningLevel};

use crate::support::{assert_writes_shape, fixture, from_value, with_unknown_fields};

#[test]
fn reads_the_machines_page() {
    let page: MachinesPage = assert_writes_shape("machines.json", &[]);
    let [pc, mac] = page.machines.as_slice() else {
        panic!("two machines")
    };
    assert_eq!(pc.state, MachineState::Warn);
    assert_eq!(pc.cpu.as_ref().unwrap().cores, Some(8.0));
    assert_eq!(pc.gpus[0].cores, None);
    let [server, unnamed] = pc.gpus[0].procs.as_slice() else {
        panic!("two processes hold the GPU's memory")
    };
    assert_eq!(server.holder(), "llama-server · example.service");
    assert_eq!(unnamed.holder(), "pid 42");
    assert_eq!(
        pc.runner
            .as_ref()
            .unwrap()
            .cap
            .as_ref()
            .unwrap()
            .by
            .as_deref(),
        Some("MemoryMax")
    );
    assert_eq!(pc.sessions[0].used5h, Some(0.25));
    assert_eq!(pc.history.as_ref().map(Vec::len), Some(1));
    assert_eq!(pc.warnings[0].level, WarningLevel::Warn);
    assert_eq!(mac.state, MachineState::Unknown);
    assert!(mac.memory.is_none());
    assert_eq!(mac.history, Some(Vec::new()), "asked for, none yet: `[]`");
    assert!((page.thresholds.unwrap().max_load_per_core - 1.5).abs() < f64::EPSILON);
}

#[test]
fn the_machines_page_tolerates_unknown_fields() {
    let mut body = fixture("machines.json");
    with_unknown_fields(&mut body);
    assert_eq!(
        from_value::<MachinesPage>(body),
        from_value::<MachinesPage>(fixture("machines.json"))
    );
}
