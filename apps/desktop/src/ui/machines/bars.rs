//! A machine's resource bars: processor, memory, graphics, disks, battery, and the session
//! runner's memory cap.

use gpui_kit::prelude::*;
use gpui_kit::{Div, FontWeight, div, px};

use crate::format::{gigabytes, minutes_left, percent, whole};
use crate::ui::palette::Pal;
use crate::ui::widgets::bar;
use emberhut_api::machines::{Battery, Cpu, Disk, Gpu, Machine, Memory};

/// The processor, memory, graphics, disk and battery bars, and the session runner's line.
pub(super) fn resources(p: &Pal, machine: &Machine, card: Div) -> Div {
    let card = card
        .children(machine.cpu.as_ref().map(|cpu| cpu_bar(p, cpu)))
        .children(machine.memory.as_ref().map(|memory| memory_bar(p, memory)))
        .children(machine.gpus.iter().map(|gpu| gpu_bar(p, gpu)))
        .children(machine.disks.iter().map(|disk| disk_bar(p, disk)))
        .children(
            machine
                .battery
                .as_ref()
                .map(|battery| battery_bar(p, battery)),
        );
    runner(p, machine, card)
}

fn cpu_bar(p: &Pal, cpu: &Cpu) -> Div {
    let cores = cpu.cores.unwrap_or(0.);
    let load = cpu.load1.unwrap_or(0.);
    let mut parts = Vec::new();
    if cores > 0. {
        parts.push(format!("load {load:.1} of {} cores", whole(cores)));
    }
    if let Some(usage) = cpu.usage {
        parts.push(format!("{} busy", percent(usage)));
    }
    let share = if cores > 0. {
        load / cores
    } else {
        cpu.usage.unwrap_or(0.)
    };
    bar(p, "CPU", share, parts.join(" · "), None, None)
}

fn memory_bar(p: &Pal, memory: &Memory) -> Div {
    let total = memory.total.unwrap_or(0.);
    let used = memory
        .available
        .map(|free| total - free)
        .or(memory.used)
        .unwrap_or(0.);
    let mut notes = Vec::new();
    if let Some(pressure) = memory
        .pressure
        .as_deref()
        .filter(|pressure| *pressure != "normal")
    {
        notes.push(format!("Memory pressure: {pressure}"));
    }
    if memory.swap_used.unwrap_or(0.) > 0. {
        notes.push(format!(
            "swap {} of {}",
            gigabytes(memory.swap_used),
            gigabytes(memory.swap_total)
        ));
    }
    let numbers = format!(
        "{} of {} · {} free",
        gigabytes(Some(used)),
        gigabytes(Some(total)),
        gigabytes(memory.available)
    );
    bar(
        p,
        "Memory",
        ratio(used, total),
        numbers,
        Some(notes.join(" · ")),
        None,
    )
}

fn gpu_bar(p: &Pal, gpu: &Gpu) -> Div {
    let shared = gpu.shared == Some(true);
    let mut name = gpu
        .name
        .clone()
        .unwrap_or_else(|| format!("GPU {}", gpu.index.unwrap_or(0)))
        .replace("NVIDIA GeForce ", "")
        .replace("NVIDIA ", "");
    if shared {
        name.push_str(" GPU");
    }
    if let Some(cores) = gpu.cores {
        name = format!("{name} · {cores} cores");
    }
    let (total, used) = (gpu.mem_total.unwrap_or(0.), gpu.mem_used.unwrap_or(0.));
    let mut notes = Vec::new();
    if let Some(util) = gpu.util {
        notes.push(format!("{} busy", percent(util)));
    }
    if let Some(temp) = gpu.temp {
        notes.push(format!("{} °C", whole(temp)));
    }
    if let Some(power) = gpu.power {
        notes.push(format!("{} W", whole(power)));
    }
    let (used_text, total_text) = (gigabytes(Some(used)), gigabytes(Some(total)));
    let numbers = if shared {
        format!("{used_text} of {total_text} unified memory")
    } else {
        format!("VRAM {used_text} of {total_text}")
    };
    let memory = bar(
        p,
        name,
        ratio(used, total),
        numbers,
        Some(notes.join(" · ")),
        None,
    );
    let busy = gpu
        .util
        .map(|util| bar(p, "GPU busy", util, percent(util), None, None).text_size(px(12.)));
    // What holds its memory: `llama-server · example.service   1.0 GB`.
    let holders = gpu.procs.iter().map(|process| {
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(4.))
            .text_size(px(11.5))
            .text_color(p.muted)
            .child(div().flex_1().min_w_0().truncate().child(process.holder()))
            .child(gigabytes(process.mem))
    });
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(memory)
        .children(busy)
        .children(holders)
}

fn disk_bar(p: &Pal, disk: &Disk) -> Div {
    let (total, free) = (disk.total.unwrap_or(0.), disk.free.unwrap_or(0.));
    let numbers = format!(
        "{} of {} · {} free",
        gigabytes(Some(total - free)),
        gigabytes(Some(total)),
        gigabytes(Some(free))
    );
    let label = disk.path.clone().unwrap_or_else(|| "Disk".to_owned());
    bar(p, label, ratio(total - free, total), numbers, None, None)
}

fn battery_bar(p: &Pal, battery: &Battery) -> Div {
    let charge = battery.percent.unwrap_or(0.);
    let mut notes = vec![if battery.plugged == Some(true) {
        "plugged in".to_owned()
    } else {
        battery
            .state
            .clone()
            .unwrap_or_else(|| "on battery".to_owned())
    }];
    if let Some(minutes) = battery.minutes_left {
        notes.push(minutes_left(minutes));
    }
    let tone = if charge < 20. {
        p.danger
    } else if charge < 40. {
        p.warn
    } else {
        p.success
    };
    let numbers = format!("{}%", whole(charge));
    bar(
        p,
        "Battery",
        charge / 100.,
        numbers,
        Some(notes.join(" · ")),
        Some(tone),
    )
}

/// `part` of `whole`, 0 when the whole is unknown.
fn ratio(part: f64, whole: f64) -> f64 {
    if whole > 0. { part / whole } else { 0. }
}

/// The session runner: its memory cap as a bar, or a line of counts.
fn runner(p: &Pal, machine: &Machine, card: Div) -> Div {
    let Some(runner) = &machine.runner else {
        return card;
    };
    let mut parts = Vec::new();
    if let Some(running) = runner.sessions {
        parts.push(format!("{running} running"));
    }
    if let Some(queued) = runner.queued.filter(|queued| *queued > 0) {
        parts.push(format!("{queued} queued"));
    }
    if let Some(open) = runner.open {
        parts.push(format!("{open} open"));
    }
    if let Some(agents) = runner.claude {
        parts.push(match runner.claude_rss {
            Some(rss) => format!("{agents} agent processes ({})", gigabytes(Some(rss))),
            None => format!("{agents} agent processes"),
        });
    }
    let cap = runner
        .cap
        .as_ref()
        .filter(|cap| cap.max.or(cap.high).unwrap_or(0.) > 0.);
    if let Some(cap) = cap {
        let limit = cap.max.or(cap.high).unwrap_or(1.);
        let mut notes = Vec::new();
        if let (Some(_), Some(high)) = (cap.max, cap.high) {
            notes.push(format!("slows at {}", gigabytes(Some(high))));
        }
        if let Some(by) = &cap.by {
            notes.push(format!("by {by}"));
        }
        notes.extend(parts);
        let numbers = format!("{} of {}", gigabytes(cap.used), gigabytes(Some(limit)));
        card.child(bar(
            p,
            "Runner's memory cap",
            cap.used.unwrap_or(0.) / limit,
            numbers,
            Some(notes.join(" · ")),
            None,
        ))
    } else if parts.is_empty() {
        card
    } else {
        card.child(
            div()
                .flex()
                .text_size(px(13.))
                .child(div().font_weight(FontWeight::MEDIUM).child("Runner"))
                .child(div().flex_1())
                .child(div().text_color(p.muted).child(parts.join(" · "))),
        )
    }
}
