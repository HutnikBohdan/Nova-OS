# Nova OS — критерії фінального приймання

> Оновлення перевірки 2026-07-14: актуальний BIOS image реально завантажив інтегровані desktop/installer/AI/runtime/hardware/build/compositor/VFS/TLS/platform/media/NovaFS/loader/productivity/security/deployment/browser/socket/USB/locale/PKI/VirtIO/compiler cores. Додано Nova Guardian: persistent append-only hash-chain journal, CRC, durable inverse snapshot, commit/reboot recovery та fault-injected power-loss rollback. Boot gates включають `NOVA_GUARDIAN_TRANSACTION_COMMITTED_OK`, `NOVA_GUARDIAN_RECOVERY_STATE_OK` і `NOVA_GUARDIAN_POWERLOSS_ROLLBACK_OK`. Це доводить конкретні інтегровані сценарії, але ще не є доказом повної комерційної ОС або підтримки реального обладнання.

Nova OS не називається фінальною, доки кожен пункт нижче не має відтворюваного
доказу у завантаженому BIOS- та UEFI-образі. Модульний тест доводить коректність
модуля, але не замінює інтеграційний тест обладнання або процесу.

| Вимога | Поточний доказ | Стан |
|---|---|---|
| Власне ядро Rust завантажується | `NOVA_OS_BOOT_OK` у QEMU BIOS та UEFI | перевірено для BIOS і UEFI |
| Україномовна стільниця | Inter/Cyrillic atlas; PS/2 українська розкладка; команда `стан` виконана в QEMU | runtime текст перевірено, потрібен фінальний візуальний аудит |
| Файли переживають перезапуск | ATA PIO + NovaFS A/B snapshot; окремий VirtIO block disk записано, VM перезапущено й отримано `NOVA_VIRTIO_BLOCK_PERSISTED_OK` | перевірено на BIOS legacy IDE та VirtIO PCI; потрібен modern VirtIO/NVMe шлях |
| Ізольовані ring-3 процеси | QEMU: два окремі CR3; USER RX/RW+NX; yield+exit syscalls; `ud2` isolation; PIT IRQ; таймер вісім разів перемкнув активні CR3, і обидва процеси виконали код (`NOVA_PREEMPTIVE_CR3_SWITCH_OK`) | базове preemptive multi-process виконання перевірено; потрібен production scheduler із довільними register contexts |
| Власні драйвери | `NOVA_PCI_READY`; ATA PIO persistence; e1000 MMIO/DMA; реальний VirtIO PCI block DMA write/read (`NOVA_VIRTIO_BLOCK_RW_OK`); VirtIO protocol core | потрібні modern VirtIO, NVMe/input та ширші апаратні переривання |
| Власна мережа | `net-core`, 11 тестів; e1000 DMA TX/RX; DHCP bind, ARP, TCP handshake і HTTP response у QEMU | зовнішній DNS у поточному QEMU backend не відповів; потрібні async socket service, DNS/TLS gates |
| Власний браузер | URL/HTTP/HTML/DOM/CSS/layout; booted `NOVA_TCP_ESTABLISHED` і `NOVA_HTTP_RESPONSE_OK` | потрібні TLS, certificates, cache, scripting sandbox і повна реальна сторінка |
| Нативні програми | редактор/package core; Ring-3 програма виконує write/yield/exit і друкує український текст (`NOVA_NATIVE_APP_PROCESS_OK`) | потрібні всі програми як довгоживучі окремі процеси та повний UX |
| Локальний AI-оператор | GGUF/tokenizer/Q8/transformer inference; capability tool-loop; Nova Guardian journal, reboot recovery та реальний power-loss rollback на VirtIO | потрібні production-модель, довга пам'ять, recovery UI та ширші автономні workflow |
| Саморозробка всередині Nova | власний NovaRust compiler генерує ELF; команда `збери /Проєкти/Привіт.nv` пройшла capability Nova Guardian, прочитала source з VFS, скомпілюва ELF і виконала його в новому ring-3 CR3 з кодом 7 (`NOVA_DYNAMIC_BUILD_RING3_OK`, `NOVA_AI_SELF_HOSTED_BUILD_OK`) | потрібні повний SDK, довгоживучий editor/build UX, linker/package store та ширша мова |
| Інсталятор, A/B update, recovery | GPT planner, A/B slots, anti-rollback, dual CRC metadata, power-loss recovery; 17 тестів; core завантажено в image | потрібен реальний install-to-empty-disk та recovery runtime test |

## Обов'язкові фінальні сценарії

1. Чистий диск: інсталяція, перше завантаження, український setup, створення користувача.
2. Створення документа в Редакторі, перезапуск, читання того самого документа.
3. Збірка й запуск Rust-програми всередині Nova в окремому процесі.
4. Навмисний crash під час запису й автоматичне journal recovery без втрати старої версії.
5. Завантаження HTTPS-сторінки у Nova Browser без host-браузера.
6. AI створює проєкт, змінює файли, збирає, тестує і запускає програму через capabilities.
7. Пошкоджене оновлення відхиляється; валідне A/B-оновлення комітиться; rollback працює.
8. BIOS і UEFI smoke, keyboard/mouse/storage/network/audio/display runtime gates.

До виконання всіх сценаріїв проєкт залишається активною розробкою, а не
«фінальною ОС».
