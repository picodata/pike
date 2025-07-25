# Cargo plugin for Picodata plugins

Плагин к cargo с функциями для упрощения разработки плагинов к Пикодате.

## Установка

```bash
cargo install picodata-pike
```

## Поддерживаемые версии

| Pike    | Picodata         |
| ------- | ---------------- |
| `1.*.*` | `> 24.6, < 25.1` |
| `2.*.*` | `>= 25.1, < 26`  |

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
- `--data-dir <DATA_DIR>` - Путь к директории хранения файлов кластера. Значение по умолчанию: `./tmp`
- `--disable-install-plugins` - Отключение автоматической установки плагинов
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

#### topology.toml

```toml
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

# настройки сервисов плагинов
[plugin.sp.service.main] # в примере настройка сервиса main плагина sp
tiers = ["default"] # тиры на которых должен работать сервис

# переменные окружения, которые будут переданы каждому инстансу Picodata
# в значении переменной можно указать liquid-шаблон, # в таком случае
# переменная будет динамически вычислена для каждого инстанса отдельно
# подробнее про liquid-шаблоны: https://shopify.dev/docs/api/liquid
[enviroment]
SP_CONST_VAR = "const" # такое значение будет передано каждому инстансу без изменений
# здесь мы используем переменную из контекста шаблонов,
# для первого, например, инстанса значение будет "1"
SP_LIQUID_VAR = "{{ instance_id }}"
# а здесь используется переменная из контекста и стандартная функция plus
# результатом для первого, например, инстанса будет "4243"
SP_LIQUID_VAR2 = "{{ instance_id | plus: 4242 }}"
```

Доступные переменные контекста в шаблонах:

- `instance_id` - порядковый номер инстанса при запуске, начинается с 1

#### picodata.yaml

Пайк позволяет использовать файл конфигурации Пикодаты вместе с запущенным кластером. Пример файла сразу генерируется командами `new` и `init`. Документацию к параметрам можно найти в [документации к Пикодате](https://docs.picodata.io/picodata/stable/reference/config/).

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

Для добавления дополнительных плагинов, не входящих в текущий проект, в топологии можно указать свойство `path` для плагина:

```toml
[plugin.ext_plugin]
...
path = "../ext_plugin"

[plugin.third_party_plugin]
...
path = "third-patry/third_party_plugin.tar.gz
```

Путь к внешнему плагину должен быть относительным, разрешается относительно рабочей директории `pike` и может являтся одним из трех вариантов:
* Директория с проектом плагина (поддерживаются и cargo project и cargo workspace)
* Директория с собранными версиями плагина, которая получается в результате выполнения `pike plugin build`
* Архив с плагином, созданный через `pike plugin pack` или вручную.

Файлы такого внешнего плагина будут помещены в общую для запуска кластера директорию, находящуюся в текущем проекте. Если путь указывает на проект, то, при необходимости, он будет предварительно собран (включая все плагины воркспейса).

### `stop`

Остановить кластер можно либо комбинацией клавиш Ctrl+C в терминале, где вызывалась команда `cargo pike run`, либо в другом окне командой:

```bash
cargo pike stop --data-dir ./tmp
```

При помощи `--data-dir` указывается путь до директории с файлами кластера _(значение по умолчанию: `./tmp`)_

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

#### Доступные опции

- `--data-dir <DATA_DIR>` - Путь к директории хранения файлов кластера. Значение по умолчанию: `./tmp`
- `--plugin-path` - Путь до директории **проекта** плагина. Значение по умолчанию: `./`
- `--instance-name <INSTANCE_NAME>` - Название инстанса Пикодаты. По умолчанию игнорируется.

### `enter`

Подключения к определенному инстансу Пикодаты по его имени

```bash
cargo pike enter default_2_1
```

#### Доступные опции

- `--data-dir <DATA_DIR>` - Путь к директории хранения файлов кластера. Значение по умолчанию: `./tmp`
- `--plugin-path` - Путь до директории **проекта** плагина. Значение по умолчанию: `./`
- `--picodata-path <BINARY_PATH>` - Путь до исполняемого файла Пикодаты. Значение по умолчанию: `picodata`

### `plugin clean`

Очистка дата-каталогов пикодаты.

```bash
cargo pike clean
```

#### Доступные опции

- `--data-dir <DATA_DIR>` - Путь к директории хранения файлов кластера. Значение по умолчанию: `./tmp`
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

#### Настройка содержания архива

По умолчанию архив будет содержать `.so` файл скомпилированного плагина, manifest.yaml, папку с миграциями, а также содержимое папки _assets_.

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

#### Доступные опции

- `--debug` - Сборка и упаковка debug-версии плагина
- `--target-dir <TARGET_DIR>` - Директория собранных бинарных файлов. Значение по умолчанию: `target`
- `--plugin-path` - Путь до директории **проекта** плагина. Значение по умолчанию: `./`

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

#### Доступные опции

- `-c, --config-path <CONFIG>` - Путь к файлу конфига. Значение по умолчанию: `plugin_config.yaml`
- `--data-dir <DATA_DIR>` - Путь к директории хранения файлов кластера. Значение по умолчанию: `./tmp`
