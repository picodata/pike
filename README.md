# Cargo plugin for Picodata plugins

Плагин к cargo с функциями для упрощения разработки плагинов к Пикодате.

## Установка

```bash
cargo install picodata-pike
```

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
[tiers.default]
instances = 2
replication_factor = 2

[[tiers.default.services]]
name = "main"
plugin = "plugin_name"
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

#### config.yaml

Пайк позволяет использовать файл конфигурации Пикодаты вместе с запущенным кластером. Пример файла сразу генерируется командами `new` и `init`. Документацию к параметрам можно найти в документации к Пикодате.

#### Настройка нескольких тиров

Для настройки необходимо добавить нужные тиры в конфигурацию кластера (файл `config.yaml`). И после указать их в файле топологии topology.toml.

Пример добавления тира `inokentiy`:

```yaml
# config.yaml

cluster:
  tier:
    default:
      replication_factor: 2
    inokentiy: # новый тир
      replication_factor: 1
```

```toml
# topology.toml

# ...

[tiers.inokentiy] # новый тир
instances = 1
replication_factor = 1
```

### `stop`

Остановить кластер можно либо комбинацией клавиш Ctrl+C в терминале, где вызывалась команда `cargo pike run`, либо в другом окне командой:

```bash
cargo pike stop --data-dir ./tmp
```

При помощи `--data-dir` указывается путь до директории с файлами кластера _(значение по умолчанию: `./tmp`)_

Вывод:

```bash
[*] stopping picodata cluster, data folder: ./tmp
[*] stopping picodata instance: i_1
[*] stopping picodata instance: i_2
[*] stopping picodata instance: i_3
[*] stopping picodata instance: i_4
```

#### Доступные опции

- `--data-dir <DATA_DIR>` - Путь к директории хранения файлов кластера. Значение по умолчанию: `./tmp`

### `plugin clean`

Очистка дата-каталогов пикодаты.

```bash
cargo pike clean
```

#### Доступные опции

- `--data-dir <DATA_DIR>` - Путь к директории хранения файлов кластера. Значение по умолчанию: `./tmp`

### `plugin new`

Создание нового проекта плагина из шаблона.

```bash
cargo pike plugin new name_of_new_plugin
```

Автоматически инициализирует в проект git. Для отключения этого поведения можно воспользоваться флагом `--without-git`.

#### Доступные опции

- `--without-git` - Отключение автоматической инициализации git-репозитория
- `--workspace` - Создание проекта плагина как воркспейса

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

#### Доступные опции

- `--debug` - Сборка и упаковка debug-версии плагина
- `--target-dir <TARGET_DIR>` - Директория собранных бинарных файлов. Значение по умолчанию: `target`

### `plugin build`

Альяс для команды `cargo build`.

```bash
cargo pike plugin build
```

#### Доступные опции

- `--release` - Сборка release-версии плагина

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
