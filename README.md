# Cargo plugin for Picodata plugins

Плагин к cargo с функциями для упрощения разработки плагинов к Пикодате.

## Содержание

- [Установка](#установка)
- [Поддерживаемые версии](#поддерживаемые-версии)
- [Quickstart](#quickstart)
- [Команды](#команды)
  - [`--help`](#--help)
  - [`run`](#run)
    - [Ожидание готовности vshard](#ожидание-готовности-vshard)
    - [topology.toml](#topologytoml)
    - [picodata.yaml](#picodatayaml)
    - [Настройка нескольких тиров](#настройка-нескольких-тиров)
    - [Подключение внешних плагинов](#подключение-внешних-плагинов)
    - [Пример запуска внешнего плагина](#пример-запуска-внешнего-плагина)
  - [`stop`](#stop)
  - [`enter`](#enter)
  - [`plugin clean`](#plugin-clean)
  - [`plugin new`](#plugin-new)
  - [`plugin add`](#plugin-add)
  - [`plugin init`](#plugin-init)
  - [`plugin pack`](#plugin-pack)
    - [Имя архива](#имя-архива)
    - [Настройка содержания архива](#настройка-содержания-архива)
    - [Сведения о сборке (build_info.yaml)](#сведения-о-сборке-build_infoyaml)
  - [`plugin build`](#plugin-build)
  - [`config apply`](#config-apply)

## Установка

```bash
cargo install picodata-pike
```

## Поддерживаемые версии

| Pike    | Picodata         |
|---------|------------------|
| `1.*.*` | `> 24.6, < 25.1` |
| `2.*.*` | `>= 25.1, < 26`  |
| `3.*.*` | `>= 25.4, < 26`  |
| `4.*.*` | `>= 26`          |
| `5.*.*` | `>= 26`          |

## Quickstart

Начнем работу с новым плагином:

```bash
cargo pike plugin new test_plugin

cd test_plugin
```

Запустим кластер, конфигурацию которого можно задать в `./topology.toml`

```bash
cargo pike run
```

В вашем распоряжении окажется рабочий кластер с установленным плагином.
Остановим кластер комбинацией `Ctrl+C` или же командой `cargo pike stop` в отдельном окне.

Если вам нужно собрать архив для поставки на сервера, это можно сделать командой:

```bash
cargo pike plugin pack
```

В папке `target` появиться желанный архив.

В шаблоне нового плагина автоматически создаётся smoke-тест, который проверяет жизненный цикл плагина (установка, миграции, включение, выключение, удаление).
Для запуска smoke-теста используйте:

```bash
cargo test
```

## Команды

### `--help`

Для всех команд есть флаг `--help` выводящий справку по использованию.

```bash
cargo pike --help
```

### `run`

Запуск кластера пикодаты по файлу `topology.toml`. Автоматически запускает плагины указанные в топологии.

Пример топологии:

```toml
pre_install_sql = [
    'ALTER SYSTEM SET raft_wal_count_max = 10000;'
]

post_install_sql = [
    '''GRANT some_role to my_user;'''
]

[tier.default]
replicasets = 2
replication_factor = 2

[plugin.super_plugin]
migration_context = [
    { name = "example_name", value = "example_value" },
]

[plugin.super_plugin.service.main]
tiers = ["default"]
```

```bash
cargo pike run --topology topology.toml --data-dir ./tmp
```

Для отключения автоматической установки и включения плагинов можно использовать опцию `--disable-install-plugins`.

#### Доступные опции

- `-t, --topology <TOPOLOGY>` - Путь к файлу топологии. Значение по умолчанию: `topology.toml`
- `--data-dir <DATA_DIR>` - Путь к директории хранения файлов кластера. Значение по умолчанию: `./tmp/pike`
- `--disable-install-plugins` - Отключение автоматической установки плагинов
- `--host <IPV4>`: адрес из сети `127.0.0.0/8`, на котором слушают iproto, HTTP и pgproto всех инстансов. Без опции iproto и pgproto слушают на `127.0.0.1`, а HTTP на `0.0.0.0`. Кластеры на разных адресах (`127.0.0.2`, `127.0.0.3` и т. д.) могут использовать одни и те же порты. Переменные `PICODATA_*_LISTEN` из топологии переопределяют эту опцию.
- `--base-http-port <BASE_HTTP_PORT>` - Базовый http-порт, с которого начнут открываться http-порты отдельных инстансов. Значение по умолчанию: `8000`
- `--base-pg-port <BASE_PG_PORT>` - Базовый порт постгрес протокола, с которого начнут открываться порты отдельных инстансов. Значение по умолчанию: `5432`
- `--picodata-path <BINARY_PATH>` - Путь до исполняемого файла Пикодаты. Значение по умолчанию: `picodata`
- `--release` - Сборка и запуск релизной версии плагина
- `--target-dir <TARGET_DIR>` - Директория собранных бинарных файлов. Значение по умолчанию: `target`
- `-d, --daemon` - Запуск кластера в режиме демона
- `--disable-colors` - Отключает раскрашивание имён инстансов в разные цвета в логах
- `--plugin-path` - Путь до директории **проекта** плагина. Значение по умолчанию: `./`
- `--no-build` - Отменить сборку плагина перед стартом кластера. Значение по умолчанию: `false`
- `--config-path` - Путь к файлу конфигурации. Значение по умолчанию: `./picodata.yaml`
- `--instance-name` - Название инстанса, которого хотим запустить, если параметр не указан - запускается весь кластер
- `--with-web-auth` - Оставить включённой аутентификацию WebUI. По умолчанию Pike отключает WebUI auth для локальной разработки запросом `ALTER SYSTEM SET jwt_secret = ''`
- `--with-audit` - Включить логи аудита. По умолчанию, они отключены. Если параметр указан - каждый инстанс кластера хранит собственный лог аудита под именем `audit.log` в своей директории.
- `--wait-vshard-discovery` - Ожидать завершения vshard discovery перед запуском plugin SQL. Значение по умолчанию: включено.
- `-w, --wait-vshard-discovery-timeout <SECONDS>` - Общий таймаут ожидания окончания работы балансировщика и vshard discovery. По умолчанию: 600 секунд.
- `--wait-vshard-idle-timeout <SECONDS>` - Таймаут ожидания окончания работы балансировщика и vshard discovery при отсутствии прогресса. По умолчанию: 60 секунд.
- `--wait-instance-online-timeout` - Таймаут ожидания статуса `Online` у каждого инстанса в кластере. По умолчанию: 300 секунд.

#### Ожидание готовности vshard

Если включена опция `--wait-vshard-discovery`, после запуска инстансов Pike ждёт готовности vshard в два этапа:

1. **Ребалансировка.** Ожидание, пока в каждом тире бакеты распределятся по репликасетам поровну.
2. **Discovery.** Ожидание, пока `vshard.router` на каждом инстансе увидит актуальное распределение бакетов.

Чтобы не ждать внутренних таймеров vshard, на каждой итерации ожидания Pike принудительно будит балансировщик и discovery роутеров.

Оба этапа ограничены общим таймаутом `--wait-vshard-discovery-timeout`. Если за `--wait-vshard-idle-timeout` распределение бакетов не меняется, запуск завершается ошибкой.


Также, можно задать iproto, http и pg порты через `enviroment` в `topology.toml`, они соответсвуют названиям переменных в `picodata run --help`. В `enviroment` выставляются значения вида `<host>:<port>`, работать будут только `127.0.0.1` и `0.0.0.0`, т. к. пайк предназначен для локальной разработки


#### topology.toml

```toml
# SQL-скрипты, которые будут выполнены перед установкой плагинов.
# Полезно для установки глобальных настроек кластера, создания пользователей или таблиц.
# Рекомендуется использовать тройные кавычки ('''), чтобы не экранировать кавычки внутри SQL.
pre_install_sql = [
    'ALTER SYSTEM SET raft_wal_count_max = 10000;',
    '''CREATE USER "my_user" WITH PASSWORD 'secret';''',
    '''CREATE TABLE "init_table" ("id" INT PRIMARY KEY, "val" TEXT) DISTRIBUTED GLOBALLY;'''
]

# SQL-скрипты, которые будут выполнены после установки и включения плагинов.
# Полезно для наполнения данными таблиц, созданных миграциями плагинов,
# или выдачи ролей, созданных миграциями, пользователям, созданным на этапе pre_install_sql
# Выполняются только если установка и включение всех плагинов прошли успешно.
post_install_sql = [
    '''GRANT some_role to my_user;'''
]

# описание количества репликасетов и фактора репликации тира
# фактор репликации отвечает за количество инстансов в одном репликасете
# в примере используется тир default, указано два репликасета и фактор репликации 2,
# следовательно будет создано всего четыре инстанса
[tier.default]
replicasets = 2
replication_factor = 2

# настройки плагинов
[plugin.sp] # в примере настройки для плагина sp
# переменные которые будут подставлены в миграции
# подробнее тут: https://docs.picodata.io/picodata/24.6/architecture/plugins/#use_plugin_config
migration_context = [
    { name = "example_name", value = "example_value" },
]
# необязательный путь до YAML-файла с конфигурацией сервисов (формат как у `config apply`),
# применяется между `MIGRATE TO` и `ENABLE`;
# если ключа нет, сервисы стартуют со значениями `default_configuration` из `manifest.yaml.template`
config = "plugin_config.yaml"

# настройки сервисов плагинов
[plugin.sp.service.main] # в примере настройка сервиса main плагина sp
tiers = ["default"] # тиры на которых должен работать сервис

# переменные окружения, которые будут переданы каждому инстансу Picodata
# в значении переменной можно указать jinja-шаблон,
# в таком случае переменная будет динамически вычислена для каждого инстанса
# отдельно подробнее про jinja-шаблоны: https://github.com/mitsuhiko/minijinja/blob/main/COMPATIBILITY.md
[enviroment]
SP_CONST_VAR = "const" # такое значение будет передано каждому инстансу без изменений
# здесь мы используем переменную из контекста шаблонов,
# для первого, например, инстанса значение будет "1"
SP_TEMPLATE_VAR = "{{ instance_id }}"
# а здесь используется переменная из контекста и стандартная функция plus
# результатом для первого, например, инстанса будет "4243"
SP_TEMPLATE_VAR2 = "{{ instance_id + 4242 }}"
# выставляем http адрес инстанса пикодаты
PICODATA_HTTP_LISTEN = "127.0.0.1:{{ instance_id + 8000 }}"
```

Доступные переменные контекста в шаблонах:

- `instance_id` - порядковый номер инстанса при запуске, начинается с 1

##### Конфигурация сервисов при запуске (`config`)

Ключ `config` в секции плагина указывает путь до YAML-файла с конфигурацией сервисов. Значения из него применяются во время установки плагина, после миграций и до включения. Формат файла такой же, как у [`config apply`](#config-apply).

**Использование на уровне файлов конфигурации**

`plugin_config.yaml`:

```yaml
example_service:
  value: "it's value"
```

`topology.toml`:

```toml
[tier.default]
replicasets = 2
replication_factor = 2

[plugin.test_plugin]
config = "plugin_config.yaml"

[plugin.test_plugin.service.example_service]
tiers = ["default"]
```

Без ключа `config` значения из файла не применяются: сервисы стартуют со значениями по умолчанию из `default_configuration` в `manifest.yaml.template` плагина (в шаблоне плагина это `value: example` в секции `example_service`), `plugin_config.yaml` из шаблона плагина не подхватывается автоматически.

Путь из `config` разрешается так:

- если плагин в крейте или workspace: относительно директории плагина (как у `config apply`);
- если плагин внешний (`path = ...`): относительно директории `topology.toml`;
- абсолютный путь используется как есть.

Если файл не найден, YAML невалиден или в нём указан сервис, которого нет в топологии, `cargo pike run` завершится с ошибкой до запуска кластера.

**Использование на уровне библиотеки**

При использовании pike как библиотеки, конфигурацию одного плагина можно передать через `add_plugin_config(имя_плагина, источник)`, где источник -- путь до файла (`PathBuf`) или структура в памяти (`PluginConfigMap`). Эта опция *полностью заменяет* `config` из топологии для указанного плагина. Относительный путь разрешается так же, как `config` в `topology.toml`.

```rust
use std::{collections::HashMap, path::PathBuf};

use pike::cluster::RunParamsBuilder;
use pike::config::PluginConfigMap;

// из файла
let params = RunParamsBuilder::default()
    .topology(topology)
    .add_plugin_config("test_plugin", PathBuf::from("plugin_config.yaml"))
    .build()?;

// из памяти
let config_map: PluginConfigMap = HashMap::from([(
    "example_service".to_string(),
    HashMap::from([("value".to_string(), serde_norway::to_value("it's value")?)]),
)]);

let params = RunParamsBuilder::default()
    .topology(topology)
    .add_plugin_config("test_plugin", config_map)
    .build()?;
```

Если установка плагинов отключена (см. `RunParamsBuilder::disable_plugin_install(true)` или CLI опцию `--disable-install-plugins`), конфигурация не читается и не проверяется.

#### picodata.yaml

Пайк позволяет использовать файл конфигурации Пикодаты вместе с запущенным кластером. Пример файла сразу генерируется командами `new` и `init`. Документацию к параметрам можно найти в [документации к Пикодате](https://docs.picodata.io/picodata/stable/reference/config/).

Файл `picodata.yaml` поддерживает [Jinja-шаблоны](https://github.com/mitsuhiko/minijinja/blob/main/COMPATIBILITY.md). При запуске кластера Пайк рендерит конфиг для каждого инстанса отдельно, подставляя переменные из контекста. Доступные переменные:

- `instance_id` — порядковый номер инстанса (начиная с 1)

Пример использования:

```yaml
instance:
  name: "my-instance-{{ instance_id }}"
```

#### Настройка нескольких тиров

Для настройки необходимо указать нужные тиры в файле топологии topology.toml.

Пример добавления тира `compute`:

```toml
# topology.toml

# ...

[tier.compute] # новый тир
replicasets = 1
replication_factor = 1
```

#### Подключение внешних плагинов

Для добавления дополнительных плагинов, не входящих в текущий проект, в топологии можно указать свойство `path` для плагина.

Поддерживаются две формы пути: относительный и абсолютный.

```toml
[plugin.ext_plugin]
path = "../ext_plugin"                # относительная директория проекта

[plugin.third_party_plugin]
path = "/opt/bundles/third_plugin_0.3.1-ubuntu_focal.tar.gz"  # абсолютный путь к архиву

[plugin.home_plugin]
path = "~/Downloads/my_plugin_0.2.0-arch_rolling.tar.gz"      # путь из $HOME
```

Путь может вести на:
* Директорию с проектом плагина (поддерживаются и cargo project и cargo workspace)
* Директорию с собранными версиями плагина, которая получается в результате выполнения `pike plugin build`
* Архив с плагином, созданный через `pike plugin pack` или вручную.

Файлы внешних плагинов автоматически помещаются в рабочую директорию запуска кластера: `<data_dir>/cluster/plugins`. Эта директория затем передаётся picodata через `--share-dir`, поэтому нет необходимости иметь «родительский» плагин. Достаточно:
- положить `topology.toml` и (опционально) `picodata.yaml` в пустую папку
- указать `path` на архив/директорию внешнего плагина
- запустить `cargo pike run`

#### Пример запуска внешнего плагина

Предварительно подготавливаем плагин:

```bash
mkdir -p "/tmp/pike-manual-test"
cd "/tmp/pike-manual-test"
cargo pike plugin new my-plugin
cd my-plugin
cargo pike plugin pack
```

Путь, полученный в результате работы `pike plugin pack` нужно будет указать в topology.toml `path`.

Создаем папку my-run в домашней директории со следующим содержимым:

```
my-run/
├── topology.toml
└── picodata.yaml   (опционально)
```

`topology.toml`:

```toml
[tier.default]
replicasets = 1
replication_factor = 3

[plugin.my-plugin]
path = "/tmp/pike-manual-test/my-plugin/./target/release/my-plugin_0.1.0-cachyos_rolling.tar.gz"
migration_context = []

[plugin.my-plugin.service.example_service]
tiers = ["default"]
```

`picodata.yaml`:

```yaml
cluster:
  name: my-plugin_0.1.0-arch_rolling
  default_bucket_count: 16384
instance:
  memtx:
    memory: 2000000000
```

Запуск:

```bash
cargo pike run
```

### `stop`

Остановить кластер можно либо комбинацией клавиш Ctrl+C в терминале, где вызывалась команда `cargo pike run`, либо в другом окне командой:

```bash
cargo pike stop --data-dir ./tmp
```

При помощи `--data-dir` указывается путь до директории с файлами кластера _(значение по умолчанию: `./tmp/pike`)_

Вывод:

```bash
[*] stopping picodata cluster, data folder: ./tmp
[*] stopping picodata instance: i1
[*] stopping picodata instance: i2
[*] stopping picodata instance: i3
[*] stopping picodata instance: i4
```

Для того, чтобы остановить только один инстанс в кластере, необходимо передать его название в опцию `--instance-name`.
Пайк остановит только указанный инстанс, а остальные продолжат свое выполнение.

Например,

```bash
cargo pike stop --data-dir ./tmp --instance-name i2
```

Вывод:

```bash
[*] stopping picodata cluster instance 'i2', data folder: ./tmp/i2
[*] stopping picodata instance: i2 - OK
```

Также можно указать Unix-сигнал, который будет отправлен инстансам, при помощи опции `--signal`.
По умолчанию используется `SIGKILL`.

Например, для корректного завершения инстансов через `SIGTERM`:

```bash
cargo pike stop --data-dir ./tmp --signal SIGTERM
```

Пикодата использует timeout при graceful shutdown кластера. Таймаут задается через переменную окружения `PICODATA_INTERNAL_ON_SHUTDOWN_TIMEOUT` в секундах:

```bash
export PICODATA_INTERNAL_ON_SHUTDOWN_TIMEOUT="3.0"
```

Соответственно, при завершении кластера через `SIGTERM` инстансы будут завершаться в течение времени, указанного в переменной `PICODATA_INTERNAL_ON_SHUTDOWN_TIMEOUT`.

Команда `cargo pike stop` также использует собственный timeout ожидания завершения процесса. Он задается через опцию `--timeout`. Если кластер не завершится в течение указанного времени, инстансам будет отправлен `SIGKILL`.

Например:

```bash
cargo pike stop --data-dir ./tmp --signal SIGTERM --timeout 10
```

#### Доступные опции

- `--data-dir <DATA_DIR>` - Путь к директории хранения файлов кластера. Значение по умолчанию: `./tmp/pike`
- `--plugin-path` - Путь до директории **проекта** плагина. Значение по умолчанию: `./`
- `--instance-name <INSTANCE_NAME>` - Название инстанса Пикодаты. По умолчанию игнорируется.
- `--signal <SIGNAL>` - Unix-сигнал, который будет отправлен процессам. По умолчанию: `SIGKILL`.
- `--timeout <TIMEOUT_SECS>` - timeout ожидания завершения кластера. По умолчанию: `30 секунд`.

### `enter`

Подключения к определенному инстансу Пикодаты по его имени

```bash
cargo pike enter default_2_1
```

#### Доступные опции

- `--data-dir <DATA_DIR>` - Путь к директории хранения файлов кластера. Значение по умолчанию: `./tmp/pike`
- `--plugin-path` - Путь до директории **проекта** плагина. Значение по умолчанию: `./`
- `--picodata-path <BINARY_PATH>` - Путь до исполняемого файла Пикодаты. Значение по умолчанию: `picodata`

### `plugin clean`

Очистка дата-каталогов пикодаты.

```bash
cargo pike clean
```

#### Доступные опции

- `--data-dir <DATA_DIR>` - Путь к директории хранения файлов кластера. Значение по умолчанию: `./tmp/pike`
- `--plugin-path` - Путь до директории **проекта** плагина. Значение по умолчанию: `./`

### `plugin new`

Создание нового проекта плагина из шаблона.

```bash
cargo pike plugin new name_of_new_plugin
```

Автоматически инициализирует в проект git. Для отключения этого поведения можно воспользоваться флагом `--without-git`.

#### Доступные опции

- `--without-git` - Отключение автоматической инициализации git-репозитория
- `--workspace` - Создание проекта плагина как воркспейса

### `plugin add`

Добавление плагина в workspace. Работает только внутри директории плагина, инициализированного с флагом `--workspace`

```bash
cargo pike plugin add name_of_new_plugin
```

#### Доступные опции

- `--plugin-path` - Путь до директории **проекта** плагина. Значение по умолчанию: `./`

### `plugin init`

Создание нового проекта плагина из шаблона в текущей папке.

```bash
cargo pike plugin init
```

Автоматически инициализирует в проект git. Для отключения этого поведения можно воспользоваться флагом `--without-git`.

#### Доступные опции

- `--without-git` - Отключение автоматической инициализации git-репозитория
- `--workspace` - Создание проекта плагина как воркспейса

### `plugin pack`

Сборка всех нужных для поставки плагина файлов в один архив (для деплоя или поставки).

```bash
cargo pike plugin pack
```

Команда `plugin pack` соберёт релизную версию плагина в новый архив в директории `target` проекта.

#### Имя архива

По умолчанию, если не указана опция `--archive-name`, имя архива включает идентификатор ОС и её вариант:

```
<plugin_name>_<version>-<os_id>_<variant>.tar.gz
```

Примеры:
- `weather-cache_0.1.1-ubuntu_focal.tar.gz`
- `weather-cache_0.1.1-centos_el8.tar.gz`
- `weather-cache_0.1.1-altlinux_p9.tar.gz`

Для rolling-дистрибутивов (например, `arch`, `gentoo`, `void`) в качестве варианта используется `rolling`.
Если конкретный вариант не может быть определён, указываются безопасные значения по умолчанию.

#### Настройка содержания архива

По-умолчанию архив будет содержать `.so`/`.dylib` файл скомпилированного плагина, manifest.yaml, папку с миграциями, 
содержимое папки _assets_, а также файл со сведениями о сборке `build_info.yaml` (см. [Сведения о сборке](#сведения-о-сборке-build_infoyaml)).

Папка _assets_ нужна чтобы положить сторонние артефакты. Артефакты можно положить либо вручную, либо передать путь до них скрипту сборки `build.rs` как:

```rust
use pike::helpers::build;

fn main() {
    let params = build::ParamsBuilder::default()
        .custom_assets(vec!["./picodata.yaml"])
        .build()
        .unwrap();

    build::main(&params);
}
```

В данном примере в папку assets будет скопирован файл `picodata.yaml`, _лежащий в корне плагина_.

#### Сведения о сборке (build_info.yaml)

`plugin pack` кладет в архив файл `<plugin_name>/<version>/build_info.yaml` (рядом с `manifest.yaml`) со сведениями 
о состоянии git-репозитория, в котором находится проект плагина, на момент упаковки:

```yaml
commit: 3f2c9a1e5b7d4c8f0a6e2b1d9c7f5a3e1b0d8c6a
dirty: false
```

- `commit` - полный хэш коммита `HEAD` репозитория. Репозиторий ищется так же, как это делает `git`: вверх по дереву каталогов от `--plugin-path`.
- `dirty` - `true`, если в рабочем дереве есть незакоммиченные изменения: измененные, добавленные в индекс или 
  неотслеживаемые и не игнорируемые через `.gitignore` файлы. Иначе `false`. Состояние проверяется после сборки, 
  поэтому учитываются и изменения, сделанные самой сборкой: например, созданный или обновленный `Cargo.lock`.

Для воркспейса архивы всех плагинов-участников получают одинаковые `commit` и `dirty` репозитория корня воркспейса.

Получить хэш из архива без полной распаковки:

```bash
tar -xOzf weather-cache_0.1.1-ubuntu_focal.tar.gz weather-cache/0.1.1/build_info.yaml
```

`build_info.yaml` не добавляется в архив, если сведения о коммите получить нельзя. Упаковка при этом не прерывается, а в лог выводится предупреждение с причиной:

- `git` не установлен или не запускается;
- проект плагина не находится в git-репозитории (например, создан с `--without-git` или собирается из распакованного исходного архива);
- в репозитории нет ни одного коммита - такое состояние оставляет `cargo pike plugin new`;
- `git` отказывается работать с репозиторием, владелец которого не совпадает с текущим пользователем

Если проект создан с `--without-git` внутри другого git-репозитория, в `build_info.yaml` попадет коммит этого репозитория.

##### Ограничения

Хэш коммита не гарантирует воспроизводимость сборки:

- При `dirty: true` содержимое архива не соответствует коммиту;
- При `--no-build` хэш соответствует состоянию репозитория на момент упаковки, а не на момент сборки артефактов.

#### Доступные опции

- `--debug` - Сборка и упаковка debug-версии плагина
- `--target-dir <TARGET_DIR>` - Директория собранных бинарных файлов. Значение по умолчанию: `target`
- `--plugin-path` - Путь до директории **проекта** плагина. Значение по умолчанию: `./`
- `--no-build` - Пропустить сборку (`cargo build`) перед упаковкой. Требует, чтобы директория сборки уже существовала и имела корректную структуру (наличие `manifest.yaml` и `lib<имя_пакета>.{so|dylib}`). При отсутствии этих артефактов команда завершится с ошибкой с предложением предварительно собрать плагин или убрать `--no-build`. Значение по умолчанию: `false`
- `--archive-name <ARCHIVE_NAME>` - Явно заданное имя/путь архива. Если путь относительный — архив будет создан в `<target>/<debug|release>/<ARCHIVE_NAME>`. Если имя/путь не оканчивается на `.tar.gz`, расширение будет автоматически добавлено

### `plugin build`

Альяс для команды `cargo build`.

```bash
cargo pike plugin build
```

#### Доступные опции

- `--release` - Сборка release-версии плагина
- `--target-dir <TARGET_DIR>` - Директория собранных бинарных файлов. Значение по умолчанию: `target`
- `--plugin-path` - Путь до директории **проекта** плагина. Значение по умолчанию: `./`

### `config apply`

Применение конфигурации сервисов плагина к запущенному командой `run` кластеру пикодаты.

Пример файла конфигурации сервисов:

```yaml
# plugin_config.yaml

main: # имя сервиса
  value: changed # пример параметра конфигурации
```

```bash
cargo pike config apply
```

Тот же файл можно применить при установке плагина через ключ `config` в `topology.toml`.

#### Доступные опции

- `-c, --config-path <CONFIG>` - Путь к файлу конфига. Значение по умолчанию: `plugin_config.yaml`
- `--data-dir <DATA_DIR>` - Путь к директории хранения файлов кластера. Значение по умолчанию: `./tmp/pike`
- `--picodata-path <BINARY_PATH>` - Путь к бинарному файлу Picodata, который будет использоваться для вызова picodata admin при применении конфига. По умолчанию используется `picodata` из `$PATH`
