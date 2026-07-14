use crate::{
    console,
    fs::RamFs,
    keyboard::{self, Key},
    mouse, print, println,
};
use agent_core::{Intent, parse};
use ai_core::agent::{AgentLoop, Phase, Policy, Tool, ToolSet, Verification};
use ai_core::journal::{ActionKind, Capability, Journal, RECORD_SIZE};
use apps_core::Document;
use core::sync::atomic::{AtomicI32, Ordering};

const BUILD_NOT_RUN: i32 = i32::MIN;
static LAST_BUILD_EXIT: AtomicI32 = AtomicI32::new(BUILD_NOT_RUN);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Home,
    Files,
    Terminal,
    Ai,
    Browser,
    Settings,
    Editor,
    Packages,
}

pub fn run(fs: &mut RamFs) -> ! {
    let mut line = [0u8; 512];
    let mut len = 0usize;
    let mut mode = Mode::Home;
    let mut document = Document::new();
    loop {
        if let Some(key) = keyboard::read_key() {
            match key {
                Key::Function(1) => switch(Mode::Home, fs, &mut mode, &mut len),
                Key::Function(2) => switch(Mode::Files, fs, &mut mode, &mut len),
                Key::Function(3) => switch(Mode::Terminal, fs, &mut mode, &mut len),
                Key::Function(4) => switch(Mode::Ai, fs, &mut mode, &mut len),
                Key::Function(5) => switch(Mode::Settings, fs, &mut mode, &mut len),
                Key::Function(6) => switch(Mode::Browser, fs, &mut mode, &mut len),
                Key::Function(7) => {
                    switch(Mode::Editor, fs, &mut mode, &mut len);
                    let _ = document.replace("");
                }
                Key::Function(8) => switch(Mode::Packages, fs, &mut mode, &mut len),
                Key::Escape => switch(Mode::Home, fs, &mut mode, &mut len),
                Key::Tab => {
                    let ukrainian = keyboard::toggle_layout();
                    if matches!(mode, Mode::Terminal | Mode::Ai | Mode::Editor) {
                        println!();
                        println!(
                            "Розкладка: {}",
                            if ukrainian {
                                "українська"
                            } else {
                                "англійська"
                            }
                        );
                        prompt(mode);
                    }
                }
                Key::Enter if mode == Mode::Editor => {
                    match fs.write("/Документи/Нова-нотатка.txt", document.text().as_bytes())
                    {
                        Ok(()) => {
                            document.mark_saved();
                            let _ = crate::ata::persist(fs);
                            println!();
                            println!("Файл збережено.");
                        }
                        Err(e) => {
                            println!();
                            println!("помилка збереження: {e:?}");
                        }
                    }
                }
                Key::Enter if matches!(mode, Mode::Terminal | Mode::Ai) => {
                    println!();
                    if let Ok(input) = core::str::from_utf8(&line[..len]) {
                        if mode == Mode::Ai {
                            route_request(fs, input);
                        } else {
                            execute(fs, parse(input));
                        }
                    }
                    len = 0;
                    prompt(mode);
                }
                Key::Backspace if mode == Mode::Editor => {
                    if len > 0 {
                        let previous = previous_char_start(&line[..len]);
                        let _ = document.delete(previous..len);
                        len = previous;
                        print!("\x08");
                    }
                }
                Key::Backspace if matches!(mode, Mode::Terminal | Mode::Ai) => {
                    if len > 0 {
                        len = previous_char_start(&line[..len]);
                        print!("\x08");
                    }
                }
                Key::Char(ch) if mode == Mode::Editor => {
                    let mut encoded = [0u8; 4];
                    let text = ch.encode_utf8(&mut encoded);
                    if len + text.len() <= line.len() {
                        line[len..len + text.len()].copy_from_slice(text.as_bytes());
                        let _ = document.insert(document.text().len(), text);
                        len += text.len();
                        print!("{}", ch);
                    }
                }
                Key::Char(ch) if matches!(mode, Mode::Terminal | Mode::Ai) => {
                    let mut encoded = [0u8; 4];
                    let text = ch.encode_utf8(&mut encoded);
                    if len + text.len() <= line.len() {
                        line[len..len + text.len()].copy_from_slice(text.as_bytes());
                        len += text.len();
                        print!("{}", ch);
                    }
                }
                Key::Char(ch) if mode == Mode::Home => {
                    switch(Mode::Terminal, fs, &mut mode, &mut len);
                    let mut encoded = [0u8; 4];
                    let text = ch.encode_utf8(&mut encoded);
                    line[..text.len()].copy_from_slice(text.as_bytes());
                    len = text.len();
                    print!("{}", ch);
                }
                _ => {}
            }
        }
        if let Some(event) = mouse::poll() {
            console::move_pointer(event.x, event.y);
            if event.left_pressed
                && let Some(app) = console::hit_test(event.x, event.y)
            {
                let next = match app {
                    console::App::Home => Mode::Home,
                    console::App::Files => Mode::Files,
                    console::App::Terminal => Mode::Terminal,
                    console::App::Ai => Mode::Ai,
                    console::App::Browser => Mode::Browser,
                    console::App::Settings => Mode::Settings,
                    console::App::Editor => Mode::Editor,
                    console::App::Packages => Mode::Packages,
                };
                switch(next, fs, &mut mode, &mut len);
                console::move_pointer(event.x, event.y);
            }
        }
        core::hint::spin_loop();
    }
}

fn switch(next: Mode, fs: &RamFs, mode: &mut Mode, len: &mut usize) {
    *mode = next;
    *len = 0;
    match next {
        Mode::Home => console::show_home(),
        Mode::Files => {
            console::show_files();
            let mut count = 0usize;
            fs.paths(|path| {
                console::file_item(path, count);
                count += 1;
            });
            console::finish_files();
        }
        Mode::Terminal => {
            console::show_terminal();
            println!("Оболонка Nova 0.2  |  повні системні можливості");
            prompt(next);
        }
        Mode::Ai => {
            console::show_ai(guardian_ui_state(fs));
            println!("Приклади: прочитай /ПРОЧИТАЙ-МЕНЕ.txt");
            println!("запиши /нотатка.txt привіт");
            prompt(next);
        }
        Mode::Settings => console::show_settings(),
        Mode::Browser => console::show_browser(),
        Mode::Editor => console::show_editor(),
        Mode::Packages => console::show_packages(),
    }
}

fn prompt(mode: Mode) {
    match mode {
        Mode::Ai => print!("nova ai > "),
        Mode::Terminal => print!("nova > "),
        _ => {}
    }
}

fn execute(fs: &mut RamFs, intent: Intent<'_>) {
    match intent {
        Intent::Help => {
            println!("допомога | стан | файли | прочитай /файл");
            println!("запиши /файл текст | додай /файл текст | видали /файл");
            println!("запусти echo текст | запусти about | оператор інструкція");
            println!("F1 Домівка | F2 Файли | F3 Термінал | F4 Nova AI");
            println!("F5 Параметри | F6 Браузер | F7 Редактор | F8 Програми");
        }
        Intent::Status => {
            println!("ядро: активне; стільниця: активна; файлова система: RAMFS");
            println!("оператор: повносистемний; локальну модель ще не завантажено");
        }
        Intent::List => fs.paths(|path| println!("{path}")),
        Intent::Read { path } => match fs.read(path) {
            Ok(text) => println!("{text}"),
            Err(e) => println!("помилка: {e:?}"),
        },
        Intent::Write { path, text } => {
            let outcome = fs.write(path, text.as_bytes());
            mutation_result(fs, outcome);
        }
        Intent::Append { path, text } => {
            let outcome = fs.append(path, text.as_bytes());
            mutation_result(fs, outcome);
        }
        Intent::Delete { path } => {
            let outcome = fs.delete(path);
            mutation_result(fs, outcome);
        }
        Intent::BuildRun { path } => run_project(fs, path),
        Intent::Run {
            program: "echo",
            args,
        } => println!("{args}"),
        Intent::Run {
            program: "about", ..
        } => println!("Власна програма Nova OS на Rust: about 0.2"),
        Intent::Run { program, .. } => println!("програму не знайдено: {program}"),
        Intent::Ask { prompt } => route_request(fs, prompt),
        Intent::Unknown { input } => println!("Не вдалося розпізнати команду: {input}"),
    }
}

fn run_project(fs: &RamFs, path: &str) {
    LAST_BUILD_EXIT.store(BUILD_NOT_RUN, Ordering::Release);
    let source = match fs.read(path) {
        Ok(source) => source,
        Err(error) => {
            println!("Не вдалося відкрити проєкт {path}: {error:?}");
            return;
        }
    };
    match crate::arch::run_compiled_source(source) {
        Ok(exit_code) => {
            LAST_BUILD_EXIT.store(exit_code, Ordering::Release);
            println!("Програму зібрано в Nova OS і виконано в ring-3. Код: {exit_code}");
        }
        Err(error) => println!("Збірка або запуск не вдалися: {error:?}"),
    }
}

pub fn self_hosting_boot_proof(fs: &mut RamFs) {
    route_request(fs, "збери /Проєкти/Привіт.nv");
    if LAST_BUILD_EXIT.load(Ordering::Acquire) == 7 {
        crate::serial::write_str("NOVA_AI_SELF_HOSTED_BUILD_OK\n");
    } else {
        crate::serial::write_str("NOVA_AI_SELF_HOSTED_BUILD_FAILED\n");
    }
}

fn route_request(fs: &mut RamFs, prompt: &str) {
    let parsed = parse(prompt);
    if matches!(parsed, Intent::Unknown { .. } | Intent::Ask { .. }) {
        println!("Локальну мовну модель ще не завантажено.");
        println!("Спробуйте пряму команду: прочитай, запиши, додай, видали, запусти.");
    } else {
        let (tool, mutating) = tool_for(parsed);
        let mut agent = AgentLoop::<8, 8>::new(Policy {
            tools: ToolSet::ALL,
            // The typed user command is the authorization for this bounded
            // action; background tasks use a separate policy and approvals.
            require_mutation_approval: false,
            require_verification: true,
        });
        let objective = hash64(prompt.as_bytes());
        if agent.start(objective, 0).is_err() {
            println!("Не вдалося створити захищене завдання.");
            return;
        }
        let Ok(call) = agent.propose(tool, objective, mutating) else {
            println!("Політика Nova заборонила цей інструмент.");
            return;
        };
        let mut guardian = match load_guardian(fs) {
            Some(journal) => journal,
            None => {
                println!("Журнал Nova Guardian пошкоджено. Зміни заблоковано до recovery.");
                return;
            }
        };
        let mut guarded_action = None;
        if mutating {
            let Some((kind, capability, before_hash, inverse, inverse_len)) =
                prepare_guardian_snapshot(fs, parsed)
            else {
                println!("Nova Guardian не зміг створити повний знімок до зміни.");
                return;
            };
            let Ok(action) = guardian.plan(
                objective as u32,
                kind,
                capability,
                before_hash,
                &inverse[..inverse_len],
            ) else {
                println!("Журнал Nova Guardian заповнений; потрібна архівація recovery.");
                return;
            };
            if guardian.approve(action).is_err() || !persist_guardian(fs, &guardian) {
                println!("Не вдалося надійно записати план Nova Guardian.");
                return;
            }
            guarded_action = Some((action, before_hash, inverse[0]));
        }
        println!("План прийнято. Виконую з вашими системними правами...");
        execute(fs, parsed);
        let verified = verify_intent(fs, parsed);
        let result_hash = intent_state_hash(fs, parsed, objective);
        if let Some((action, before_hash, existed)) = guarded_action {
            if guardian.applied(action, result_hash).is_err() || !persist_guardian(fs, &guardian) {
                println!(
                    "Зміна виконана, але checkpoint не записано; recovery буде запропоновано."
                );
                return;
            }
            if !verified {
                let restored = restore_guardian_snapshot(fs, parsed, existed);
                if restored
                    && guardian.revert(action, before_hash).is_ok()
                    && persist_guardian(fs, &guardian)
                {
                    println!("Перевірка не пройшла. Nova Guardian автоматично відкотила зміну.");
                } else {
                    println!("Перевірка не пройшла; потрібен recovery Nova Guardian.");
                }
                return;
            }
            if guardian.verify(action, result_hash).is_err() || !persist_guardian(fs, &guardian) {
                println!("Не вдалося зафіксувати перевірену транзакцію Nova Guardian.");
                return;
            }
            crate::serial::write_str("NOVA_GUARDIAN_TRANSACTION_COMMITTED_OK\n");
        }
        let _ = agent.execution_result(call.id, result_hash, verified);
        if !verified {
            println!("Перевірка результату не пройшла; завдання не позначено завершеним.");
            return;
        }
        if agent.verify(call.id, Verification::Accepted).is_err()
            || agent.complete(result_hash).is_err()
            || agent.phase() != Phase::Completed
        {
            println!("Не вдалося зафіксувати перевірений checkpoint.");
            return;
        }
        crate::serial::write_str("NOVA_AI_CAPABILITY_TASK_OK\n");
        println!("Завдання виконано й перевірено.");
    }
}

const GUARDIAN_JOURNALS: [&str; 4] = [
    "/system/guardian.0",
    "/system/guardian.1",
    "/system/guardian.2",
    "/system/guardian.3",
];
const GUARDIAN_UNDO: &str = "/system/guardian.undo";

pub fn guardian_boot_proof(fs: &mut RamFs) {
    const PATH: &str = "/system/guardian.proof";
    const VALUE: &str = "verified";
    let Some(recovered) = load_guardian(fs) else {
        return;
    };
    let mut recovered = recovered;
    if let Some(pending) = recovered.recovery_action().copied() {
        let mut durable = [0u8; 1536];
        let Some(total) = crate::virtio_block::read_undo(&mut durable) else {
            return;
        };
        if total < 2 {
            return;
        }
        let path_len = durable[1] as usize;
        if total < 2 + path_len || durable[0] != pending.inverse().first().copied().unwrap_or(2) {
            return;
        }
        let Ok(path) = core::str::from_utf8(&durable[2..2 + path_len]) else {
            return;
        };
        let restored_hash = if durable[0] == 1 {
            let content = &durable[2 + path_len..total];
            if fs.write(path, content).is_err() {
                return;
            }
            hash64(content)
        } else {
            let _ = fs.delete(path);
            0
        };
        if restored_hash == pending.before_hash
            && recovered.revert(pending.action_id, restored_hash).is_ok()
            && persist_guardian(fs, &recovered)
        {
            crate::serial::write_str("NOVA_GUARDIAN_POWERLOSS_ROLLBACK_OK\n");
        }
        return;
    }
    if recovered.recovery_action().is_none()
        && recovered.records().last().is_some_and(|record| {
            record.task_id == 0x4e4f_5641 && record.phase == ai_core::journal::Phase::Verified
        })
    {
        let _ = fs.write(PATH, VALUE.as_bytes());
        crate::serial::write_str("NOVA_GUARDIAN_RECOVERY_STATE_OK\n");
        return;
    }
    let intent = Intent::Write {
        path: PATH,
        text: VALUE,
    };
    let Some((kind, capability, before, inverse, inverse_len)) =
        prepare_guardian_snapshot(fs, intent)
    else {
        return;
    };
    let mut journal = recovered;
    let Ok(action) = journal.plan(
        0x4e4f_5641,
        kind,
        capability,
        before,
        &inverse[..inverse_len],
    ) else {
        return;
    };
    if journal.approve(action).is_err() || !persist_guardian(fs, &journal) {
        return;
    }
    if fs.write(PATH, VALUE.as_bytes()).is_err() {
        return;
    }
    let after = hash64(VALUE.as_bytes());
    if journal.applied(action, after).is_err() || !persist_guardian(fs, &journal) {
        return;
    }
    #[cfg(feature = "guardian-fault-injection")]
    {
        crate::serial::write_str("NOVA_GUARDIAN_POWERLOSS_INJECTED\n");
        return;
    }
    #[cfg(not(feature = "guardian-fault-injection"))]
    {
        if journal.verify(action, after).is_ok() && persist_guardian(fs, &journal) {
            crate::serial::write_str("NOVA_GUARDIAN_TRANSACTION_COMMITTED_OK\n");
        }
    }
}

fn load_guardian(fs: &RamFs) -> Option<Journal<32>> {
    let mut slots = [[0u8; RECORD_SIZE]; 32];
    let mut local_records = 0usize;
    for (segment, path) in GUARDIAN_JOURNALS.iter().enumerate() {
        let Ok(bytes) = fs.read_bytes(path) else {
            continue;
        };
        if bytes.len() % RECORD_SIZE != 0 || bytes.len() > RECORD_SIZE * 8 {
            return None;
        }
        for (index, chunk) in bytes.chunks_exact(RECORD_SIZE).enumerate() {
            slots[segment * 8 + index].copy_from_slice(chunk);
            local_records += 1;
        }
    }
    if local_records == 0 {
        let mut durable = [0u8; RECORD_SIZE * 32];
        if let Some(len) = crate::virtio_block::read_guardian(&mut durable) {
            if len % RECORD_SIZE != 0 {
                return None;
            }
            for (slot, chunk) in slots
                .iter_mut()
                .zip(durable[..len].chunks_exact(RECORD_SIZE))
            {
                slot.copy_from_slice(chunk);
            }
        }
    }
    Journal::recover(&slots).ok()
}

fn guardian_ui_state(fs: &RamFs) -> console::GuardianUiState {
    let Some(journal) = load_guardian(fs) else {
        return console::GuardianUiState::RecoveryRequired;
    };
    if journal.recovery_action().is_some() {
        console::GuardianUiState::RecoveryRequired
    } else if journal.records().is_empty() {
        console::GuardianUiState::Ready
    } else {
        console::GuardianUiState::VerifiedHistory
    }
}

fn persist_guardian(fs: &mut RamFs, journal: &Journal<32>) -> bool {
    let mut durable = [0u8; RECORD_SIZE * 32];
    for (segment, path) in GUARDIAN_JOURNALS.iter().enumerate() {
        let start = segment * 8;
        let end = journal.records().len().min(start + 8);
        if start >= end {
            let _ = fs.delete(path);
            continue;
        }
        let mut bytes = [0u8; RECORD_SIZE * 8];
        for index in start..end {
            let mut slot = [0u8; RECORD_SIZE];
            if journal.encode_slot(index, &mut slot).is_err() {
                return false;
            }
            durable[index * RECORD_SIZE..(index + 1) * RECORD_SIZE].copy_from_slice(&slot);
            bytes[(index - start) * RECORD_SIZE..(index - start + 1) * RECORD_SIZE]
                .copy_from_slice(&slot);
        }
        if fs
            .write(path, &bytes[..(end - start) * RECORD_SIZE])
            .is_err()
        {
            return false;
        }
    }
    let ata = crate::ata::persist(fs).is_ok();
    let virtio =
        crate::virtio_block::write_guardian(&durable[..journal.records().len() * RECORD_SIZE]);
    ata || virtio
}

fn prepare_guardian_snapshot(
    fs: &mut RamFs,
    intent: Intent<'_>,
) -> Option<(ActionKind, Capability, u64, [u8; 44], usize)> {
    let mut inverse = [0u8; 44];
    let (kind, path) = match intent {
        Intent::Write { path, .. } | Intent::Append { path, .. } => {
            (ActionKind::FileWrite, Some(path))
        }
        Intent::Delete { path } => (ActionKind::FileDelete, Some(path)),
        Intent::BuildRun { .. } => (ActionKind::Build, None),
        Intent::Run { .. } => (ActionKind::Launch, None),
        _ => (ActionKind::Setting, None),
    };
    let before_hash;
    if let Some(path) = path {
        let mut snapshot = [0u8; 1024];
        if let Ok(previous) = fs.read_bytes(path) {
            let len = previous.len();
            snapshot[..len].copy_from_slice(previous);
            before_hash = hash64(previous);
            inverse[0] = 1;
            fs.write(GUARDIAN_UNDO, &snapshot[..len]).ok()?;
            let mut durable = [0u8; 1074];
            durable[0] = 1;
            durable[1] = path.len() as u8;
            durable[2..2 + path.len()].copy_from_slice(path.as_bytes());
            durable[2 + path.len()..2 + path.len() + len].copy_from_slice(&snapshot[..len]);
            let ata = crate::ata::persist(fs).is_ok();
            let virtio = crate::virtio_block::write_undo(&durable[..2 + path.len() + len]);
            if !ata && !virtio {
                return None;
            }
        } else {
            before_hash = 0;
            inverse[0] = 0;
            let _ = fs.delete(GUARDIAN_UNDO);
            let mut durable = [0u8; 50];
            durable[1] = path.len() as u8;
            durable[2..2 + path.len()].copy_from_slice(path.as_bytes());
            let ata = crate::ata::persist(fs).is_ok();
            let virtio = crate::virtio_block::write_undo(&durable[..2 + path.len()]);
            if !ata && !virtio {
                return None;
            }
        }
    } else {
        before_hash = 0;
        inverse[0] = 2;
    }
    Some((
        kind,
        if path.is_some() {
            Capability::Files
        } else if kind == ActionKind::Build {
            Capability::Build
        } else {
            Capability::Processes
        },
        before_hash,
        inverse,
        1,
    ))
}

fn restore_guardian_snapshot(fs: &mut RamFs, intent: Intent<'_>, existed: u8) -> bool {
    let path = match intent {
        Intent::Write { path, .. } | Intent::Append { path, .. } | Intent::Delete { path } => path,
        _ => return true,
    };
    let result = if existed == 1 {
        let mut snapshot = [0u8; 1024];
        let len = if let Ok(previous) = fs.read_bytes(GUARDIAN_UNDO) {
            let len = previous.len();
            snapshot[..len].copy_from_slice(previous);
            len
        } else {
            let mut durable = [0u8; 1536];
            let Some(total) = crate::virtio_block::read_undo(&mut durable) else {
                return false;
            };
            if total < 2
                || durable[0] != 1
                || durable[1] as usize != path.len()
                || &durable[2..2 + path.len()] != path.as_bytes()
            {
                return false;
            }
            let len = total - 2 - path.len();
            snapshot[..len].copy_from_slice(&durable[2 + path.len()..total]);
            len
        };
        fs.write(path, &snapshot[..len])
    } else {
        fs.delete(path).or(Ok(()))
    };
    result.is_ok() && crate::ata::persist(fs).is_ok()
}

fn intent_state_hash(fs: &RamFs, intent: Intent<'_>, fallback: u64) -> u64 {
    match intent {
        Intent::Write { path, .. } | Intent::Append { path, .. } => {
            fs.read_bytes(path).map(hash64).unwrap_or(0)
        }
        Intent::Delete { path } => {
            if fs.read_bytes(path).is_err() {
                0
            } else {
                fallback
            }
        }
        _ => fallback,
    }
}

fn tool_for(intent: Intent<'_>) -> (Tool, bool) {
    match intent {
        Intent::Read { .. } => (Tool::ReadFile, false),
        Intent::Write { .. } | Intent::Append { .. } | Intent::Delete { .. } => {
            (Tool::WriteFile, true)
        }
        Intent::List => (Tool::ListDirectory, false),
        Intent::BuildRun { .. } => (Tool::BuildProject, true),
        Intent::Run { .. } => (Tool::RunProgram, true),
        _ => (Tool::SystemControl, false),
    }
}

fn verify_intent(fs: &RamFs, intent: Intent<'_>) -> bool {
    match intent {
        Intent::Read { path } => fs.read(path).is_ok(),
        Intent::Write { path, text } => fs.read(path) == Ok(text),
        Intent::Append { path, text } => fs.read(path).is_ok_and(|value| value.ends_with(text)),
        Intent::Delete { path } => fs.read(path).is_err(),
        Intent::BuildRun { .. } => LAST_BUILD_EXIT.load(Ordering::Acquire) != BUILD_NOT_RUN,
        Intent::Run { program, .. } => matches!(program, "echo" | "about"),
        Intent::Unknown { .. } | Intent::Ask { .. } => false,
        _ => true,
    }
}

fn hash64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

fn result(result: Result<(), crate::fs::FsError>) {
    match result {
        Ok(()) => println!("Готово."),
        Err(e) => println!("помилка: {e:?}"),
    }
}

fn mutation_result(fs: &RamFs, outcome: Result<(), crate::fs::FsError>) {
    if outcome.is_ok() {
        let _ = crate::ata::persist(fs);
    }
    result(outcome);
}

fn previous_char_start(bytes: &[u8]) -> usize {
    let mut at = bytes.len().saturating_sub(1);
    while at > 0 && bytes[at] & 0b1100_0000 == 0b1000_0000 {
        at -= 1;
    }
    at
}
