# Nova OS production-storage r3 — незалежний acceptance-аудит

Дата аудиту: 2026-07-15  
Вхідний архів: `Nova-OS-production-storage-r3-FULL-VM-KIT-2026-07-15.zip`  
SHA-256: `898ABAD4B16B08D978522E027F8C04C3E4E33ECAA213FCFFFA5316D11D803E39`

## Вердикт

R3 є корисним development source candidate, але переданий snapshot не є
збірним, boot-tested, installable або persistent-storage-ready релізом.
Його не можна переносити поверх перевіреного kernel tree цілком, оскільки це
видалить наявні FXSAVE/SSE isolation та checked user-copy механізми.

## Підтверджено

- ZIP повністю розпаковується, а його зовнішній SHA-256 збігається.
- Offline `cargo metadata --locked --offline --no-deps` проходить.
- Workspace template має розмір 128 MiB і SHA-256
  `74CBCB388E8F8C621602767A35EFF8F316212A5435E1C668B6EE3537C1B380F6`.
- Модельні тести `novafs-core`, `installer-core`, `deployment-core` та
  `storage-core` проходять у вихідному snapshot до integrated build boundary.
- ELF/W^X loader є змістовним donor-модулем: його 12 тестів проходять.
- PID 1, service manifests, private CR3 і process-local capability tables
  реалізовані на рівні source candidate.

## Блокери оригінального snapshot

1. `cargo fmt --all -- --check` завершується помилкою.
2. `runtime-core` не компілюється через три `E0502`:
   `service_abi.rs:159`, `storage_rpc.rs:97`, `storage_rpc.rs:161`.
3. Після локального мінімального виправлення цих трьох borrow-помилок один
   scheduler corruption test падає через debug assertion до fail-closed check.
4. Після виправлення тесту `nova-init` збирається як `ET_DYN`, хоча kernel
   build contract вимагає `ET_EXEC`.
5. Після target-specific static/no-PIE налаштування kernel відкриває ще дев'ять
   compile errors: відсутній `PAGE_SIZE`, відсутні `Eq/PartialEq` для
   `UserSpaceError` і неоднозначні виклики `map_zeroed_page`.
6. Default та legacy Nova images у переданому snapshot не збираються; BIOS і
   UEFI acceptance виконати неможливо.
7. Evidence patch не накладається на заявлений base commit `cde4268c`: його
   контекст очікує файли/члени workspace, яких у цьому commit немає.

## Storage/VFS реальність

- `nova-vfsd` після bootstrap лише викликає yield; VFS RPC, paths, mounts і
  file handles не реалізовані.
- `nova-storaged` готує mount і потім також yield-loop; production RPC loop
  відсутній, а `storage_rpc` типи не мають production consumer.
- Workspace image містить GPT та два NovaFS superblocks, але bitmap, inode
  table і journal нульові, хоча superblock оголошує `root_inode = 1`.
- Kernel читає лише primary GPT і не виконує fallback/cross-check backup GPT.
- Installer/A-B/recovery мають моделі й traits, але не мають production
  реалізацій GPT writer, filesystem formatter, firmware writer, installer ELF,
  recovery image або persistent boot selector.
- Немає QEMU forced-poweroff/reboot-persistence та фізичних NVMe/AHCI тестів.

## Delivery integrity

- Малий delivery manifest: 12/12 записів збігаються.
- `SOURCE-SHA256SUMS`: два некоректні записи, включно із self-hash.
- `BUILD-KIT-SHA256SUMS`: 20 mismatch і 50 поточних файлів відсутні в manifest.
- Boot image у комплекті відсутній; Workspace RAW є data disk, не ОС.
- One-click launcher не fail-closed: missing markers, panic і QEMU failure є
  warnings; QEMU version не pinned; downloaded rustup-init не перевіряється
  очікуваним hash.

## Безпечна інтеграція

R3 слід переносити блоками в `codex/production-storage-r3` поверх перевіреного
commit із FPU/SSE та user-copy hardening. Першими donor-блоками мають бути
loader/ABI/model modules. Kernel `arch`, scheduler, process і user-space файли
потребують ручного merge. До runtime acceptance необхідні capability-protected
IPC endpoints, blocking wait/yield, real storaged/VFS RPC, initialized root
namespace, backup-GPT recovery і fail-closed BIOS/UEFI/persistence gates.
