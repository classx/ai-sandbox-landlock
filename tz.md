Landlock даёт тебе то же «видит только проект», но без root и без eBPF: процесс сам на себя вешает LSM‑ограничения и уже не может их снять. [docs.kernel](https://docs.kernel.org/security/landlock.html)

## 1. Кратко про Landlock

- Это LSM, доступен с ядра 5.13+, включается через `CONFIG_SECURITY_LANDLOCK` и `lsm=...landlock...` в cmdline ядра. [man7](https://man7.org/linux/man-pages/man7/landlock.7.html)
- Любой непривилегированный процесс может:
  - создать ruleset (какие типы доступа мы контролируем — чтение, запись, exec и т.п.), [docs.kernel](https://docs.kernel.org/userspace-api/landlock.html)
  - добавить правила вида «под этим каталогом разрешены такие права»,
  - «зажать» себя (`landlock_restrict_self`), после чего он и все его дети живут в этих рамках. [kernel](https://www.kernel.org/doc/html/v6.0/userspace-api/landlock.html)

Важно: Landlock **только ужесточает** права, отменить или ослабить их нельзя. [docs.kernel](https://docs.kernel.org/security/landlock.html)

## 2. Модель под твою задачу

Цель: процесс Copilot/LLM‑backend видит **только** проект (и, по необходимости, ещё пару директорий), всё остальное ФС — запрещено для чтения/записи/exec.

Сценарий:

- Ты запускаешь свой Rust‑launcher (`ai-sandbox-landlock`),
- он:
  - создаёт ruleset с интересующими типами доступа (например, read, write, execute, truncate), [docs.kernel](https://docs.kernel.org/userspace-api/landlock.html)
  - добавляет в него правила `PATH_BENEATH`:
    - `project_root` с `READ_FILE | READ_DIR | EXECUTE` (и, если нужно, `WRITE_FILE`),
    - опциональные каталоги (`/usr`, `/lib`) для чтения,
  - вызывает `landlock_restrict_self`,
  - уже **внутри** этих ограничений делает `execve` VSCode backend’а / Copilot‑процесса. [kernel](https://www.kernel.org/doc/html/v6.0/userspace-api/landlock.html)

Любая попытка открыть файл вне этих деревьев будет запрещена ядром (обычно `EPERM`/`EACCES`). [ffwde](https://ffwde.com/blog/landlock/)

## 3. Конкретные шаги (в терминах syscalls)

Упрощённо (C‑API, но ты будешь это оборачивать в Rust или использовать crate):

1. Проверка Landlock:

   - `landlock_create_ruleset(NULL, 0, LANDLOCK_CREATE_RULESET_VERSION)` — узнать ABI‑версию, или через `landlock` crate — `Compatible`. [landlock](https://landlock.io/rust-landlock/landlock/)

2. Создание ruleset:

   - заполняешь битмаску `handled_access_fs` (что вообще собираешься контролировать), например:
     - `LANDLOCK_ACCESS_FS_EXECUTE`,
     - `LANDLOCK_ACCESS_FS_READ_FILE`, `READ_DIR`,
     - `LANDLOCK_ACCESS_FS_WRITE_FILE`, `REMOVE_FILE`, и т.п. [docs.kernel](https://docs.kernel.org/userspace-api/landlock.html)
   - `ruleset_fd = landlock_create_ruleset(&ruleset_attr, sizeof(ruleset_attr), 0)`. [kernel](https://www.kernel.org/doc/html/v6.0/userspace-api/landlock.html)

3. Добавление правил `PATH_BENEATH`:

   - Открываешь директорию проекта: `project_fd = open(project_path, O_PATH | O_CLOEXEC)`. [kernel](https://www.kernel.org/doc/html/v6.0/userspace-api/landlock.html)
   - Заполняешь `struct landlock_path_beneath_attr`:
     - `parent_fd = project_fd`,
     - `allowed_access` — битмаска прав, например:
       - проект: `READ_FILE | READ_DIR | EXECUTE | WRITE_FILE`,
       - `/usr`: только `READ_FILE | READ_DIR | EXECUTE`. [docs.kernel](https://docs.kernel.org/userspace-api/landlock.html)
   - `landlock_add_rule(ruleset_fd, LANDLOCK_RULE_PATH_BENEATH, &attr, 0)`. [manpages.ubuntu](https://manpages.ubuntu.com/manpages/questing/man7/landlock.7.html)

4. Применение к себе:

   - `landlock_restrict_self(ruleset_fd, 0)`. [kernel](https://www.kernel.org/doc/html/v6.0/userspace-api/landlock.html)
   - Закрываешь `ruleset_fd`.
   - Дальше **в этом же процессе** вызываешь `execve`, и все дочерние процессы наследуют ограничения. [docs.kernel](https://docs.kernel.org/userspace-api/landlock.html)

После этого:

- только файлы/директории, лежащие «под» указанными `parent_fd`, доступны согласно `allowed_access`;
- всё остальное, для чего тип доступа входит в `handled_access_fs`, будет запрещено. [kernel](https://www.kernel.org/doc/html/v6.8/userspace-api/landlock.html)

## 4. Rust‑стек

Есть готовый crate `landlock`, который закрывает всю работу с syscalls:

- `landlock` предоставляет builder‑API для `Ruleset`, `PathBeneath`, и учитывает версии ABI ядра. [lib](https://lib.rs/crates/landlock)
- Пример из документации:
  - создаёшь `Ruleset::default()`,
  - добавляешь `path_beneath_rules()`,
  - вызываешь `ruleset.restrict_self()` — дальше можешь делать `Command::new("code")...` уже в песочнице. [landlock](https://landlock.io/rust-landlock/landlock/)

Для твоего случая логика Rust‑launcher’а:

1. Спарсить конфиг/CLI (список проектов, какие каталоги разрешены, какие права).
2. Через `landlock` crate собрать `Ruleset`:
   - добавить `PathBeneath` для каждого корня с нужными правами.
3. Вызвать `.restrict_self()`.
4. Запустить нужную команду (VSCode / backend) как дочерний процесс (или через `exec` в том же процессе).

## 5. Особенности и ограничения

- **Только ужесточение:** Landlock не может дать больше прав, чем уже есть по DAC/другим LSM (SELinux, AppArmor). Он только урезает доступ. [landlock](https://landlock.io)
- **Права фиксируются на момент `restrict_self`:**
  - уже открытые файловые дескрипторы сохраняют свои права и могут быть переданы другим процессам. [docs.kernel](https://docs.kernel.org/security/landlock.html)
- **Монтирования:** Landlock работает по файловой иерархии и хорошо дружит с bind‑mount; ограничения распространяются и на зеркала дерева через bind. [kernel](https://www.kernel.org/doc/html/v6.8/userspace-api/landlock.html)
- **Версия ABI:** старые ядра поддерживают меньше типов доступа (без удаления директорий/файлов и т.п.), нужно учитывать через версионность в crate. [landlock](https://landlock.io/rust-landlock/landlock/)

## 6. Как это смотрится относительно варианта с namespaces

По отношению к варианту 1 (mount‑namespace):

- Landlock не требует root/CAP_SYS_ADMIN: процесс сам себя ограничивает. [man7](https://man7.org/linux/man-pages/man7/landlock.7.html)
- Он **не меняет видимую структуру ФС**, только запрещает операции; процесс всё может «видеть» (список файлов), но не обязательно может их читать. [landlock](https://landlock.io)
- Часто удобно **комбинировать**:
  - mount‑namespace скрывает «лишние» куски,
  - Landlock добавляет жёсткий LSM‑стопер на случай, если ты что‑то недомонтировал или есть хитрые пути (bind‑mount’ы, symlink’и). [manpages.ubuntu](https://manpages.ubuntu.com/manpages/questing/man7/landlock.7.html)




```yaml
# ai-sandbox-landlock.yaml — профили под Landlock LSM

version: 1

profiles:
  vscode-copilot:
    description: "VSCode + Copilot, доступ только к проектам + системные каталоги"

    # Корневые директории, доступ к которым разрешен (PATH_BENEATH)
    access_roots:
      projects:
        paths:
          - ~/dev/myproj
          - ~/dev/shared-lib
        # Права для проекта (битмаска Landlock)
        permissions:
          read_file: true
          read_dir: true
          execute: true
          write_file: true     # если ИИ может генерить файлы
          remove_file: true    # удаление файлов
          remove_dir: false    # директории не удаляем

      system:
        paths:
          - /usr
          - /lib
          - /lib64
        permissions:
          read_file: true
          read_dir: true
          execute: true
          # write_*: false по умолчанию

      cache:
        paths:
          - ~/.ai-sandbox/cache
        permissions:
          read_file: true
          read_dir: true
          write_file: true
          remove_file: true

    # Что вообще контролируем (handled_access_fs)
    control_access:
      read_file: true
      read_dir: true
      execute: true
      write_file: true
      remove_file: true
      remove_dir: false
      truncate: true
      # lock: false  # file locking (если нужно)

    # Команда для запуска
    command:
      binary: code
      args:
        - .
      working_dir: ~/dev/myproj
      env:
        HOME: ~/.ai-sandbox/vscode-home  # можно задать отдельный HOME

    # Дополнительно
    log_level: info
    dry_run: false

  copilot-backend:
    description: "LLM/Copilot сервер, только чтение проекта, без сети"

    access_roots:
      projects:
        paths:
          - ~/dev/myproj
        permissions:
          read_file: true
          read_dir: true
          execute: false      # не исполняем
          write_file: false   # только чтение

      system:
        paths:
          - /usr
          - /lib
          - /lib64
        permissions:
          read_file: true
          read_dir: true
          execute: true

    control_access:
      read_file: true
      read_dir: true
      execute: true
      # write_* и remove_* отключены — не контролируем, наследуем от DAC

    command:
      binary: /usr/bin/node
      args:
        - /opt/copilot/server.js
      working_dir: ~/dev/myproj
      env:
        NODE_ENV: production

    log_level: warn

  minimal:
    description: "Минимальный доступ, только один проект на чтение"

    access_roots:
      projects:
        paths:
          - ~/dev/test-project
        permissions:
          read_file: true
          read_dir: true
          # всё остальное запрещено

    control_access:
      read_file: true
      read_dir: true
      execute: false

    command:
      binary: /bin/bash
      args: []
```

## Как это мапится на Landlock API

Rust‑launcher читает профиль и делает:

```rust
// псевдокод
let ruleset = Ruleset::default()
  .handle_path_beneath(LandlockAccess::from_flags([
    Access::ReadFile,
    Access::ReadDir,
    Access::Execute,    // из control_access
  ]));

// для каждого root из access_roots
for root in profile.access_roots.projects.paths {
  let fd = open(root, O_PATH)?;
  let perms = Access::from_flags([
    if profile.projects.permissions.read_file { Access::ReadFile } else { /*skip*/ },
    // ...
  ]);
  ruleset.add_rule(PathBeneath::new(fd).access(perms))?;
}

// затем
ruleset.restrict_self()?;
// exec команды
```

## Использование CLI

```bash
ai-sandbox-landlock \
  --config ai-sandbox-landlock.yaml \
  --profile vscode-copilot \
  -- code .
```

**Преимущества такого YAML:**
- `control_access` — что вообще хотим контролировать (битмаска `handled_access_fs`), [docs.kernel](https://docs.kernel.org/userspace-api/landlock.html)
- `access_roots` — иерархия правил (группы с разными правами),
- Легко тестировать/дебаггить: launcher может вывести итоговый `ruleset` перед применением.

**Что запрещено по умолчанию:**
- Вне `access_roots` все операции из `control_access` блокируются.
- Внутри `access_roots` разрешено только то, что явно указано в `permissions`.