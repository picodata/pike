# Test framework for Picodata plugin

## Введение
Picotest является оберткой над `rstest`, которая позволяет запускать кластер Picodata с помощью `pike` для запуска тестов

Чтобы использовать его, добавьте в свой `Cargo.toml` файл:
```toml
[dev-dependencies]
rstest = "0.23.0"
picotest = { git = "https://github.com/picodata/pike.git", branch = "plugin_test_framework" }
```

## Использование

Макрос `#[picotest]` может быть применим как к функциям, так и к модулям

для функции:
```rust
use picotest::picotest;
use rstest::rstest;

#[picotest]
fn test_foo_bar() {
    assert_eq!("foo bar", "foo bar");
}
```

для модуля:
```rust
use picotest::picotest;

#[picotest]
mod test_mod {

    fn test_foo() {
      assert_eq!("foo", "foo");
    }

    fn test_bar() {
      assert_eq!("bar", "bar");
    }
}
```

Запустите тесты с использованием переменной RUST_TEST_THREADS=1:
```sh
RUST_TEST_THREADS=1 cargo test
```

наличие переменной `RUST_TEST_THREADS=1` важно, т.к. `rust` может запускать несколько тестов одновременно, что приводит к дублированию кластера

## Использование #[picotest] на функциях

При исполльзовании макроса на функциях, `picotest` будет создавать кластер при каждом запуске очередного теста и удалять кластер по завершению теста

## Использование #[picotest] на модулях
При использовании макроса на модуле, `picotest` автоматически пометит все фукнции которые начинаются с `test_` как `rstest` функции, кластер будет создан только 1 раз и удален по завершению всех тестов в модуле

## Возможности расширения
`picotest` является оберткой над `rstest`, поэтому поддерживает использование `fixture`

```rust
use picotest::picotest;

#[picotest]
mod test_mod {
    #[fixture]
    fn foo() -> String {
        "foo".to_string()
    }

    #[fixture]
    fn bar() -> String {
        "bar".to_string()
    }

    fn test_foo(foo: String) {
        assert_eq!(foo, "foo".to_string());
    }

    fn test_bar(bar: String) {
        assert_eq!(bar, "bar".to_string());
    }

    fn test_foo_bar(foo: String, bar: String) {
        assert_ne!(foo, bar);
    }
}
```

## Дополнительные параметры

Макрос `#[picotest]` поддерживает следующие аргументы:

* path - путь до дирректории плагина
* timeout - выждать timeout прежде чем запустить тест 


## Пользовательские хуки

Picotest поддерживает работу с хуками `before_all` и `after_all`
Для использования добавьте в свой `Cargo.toml` файл:
```toml
[dev-dependencies]
test-env-helpers = "0.2.2"
```

Пример:
```rust
use picotest::picotest;
use test_env_helpers::{after_all, before_all};

#[picotest]
#[before_all]
#[after_all]
mod test_mod {

    fn before_all() {
        todo!()
    }

    fn after_all() {
        todo!()
    }

    #[fixture]
    fn foo() -> String {
        "foo".to_string()
    }

    #[fixture]
    fn bar() -> String {
        "bar".to_string()
    }

    fn test_foo(foo: String) {
        assert_eq!(foo, "foo".to_string());
    }

    fn test_bar(bar: String) {
        assert_eq!(bar, "bar".to_string());
    }

    fn test_foo_bar(foo: String, bar: String) {
        assert_ne!(foo, bar);
    }
}
```

## Использование без макроса

Picotest позволяет создавать / удалять кластер без использования макроса `#[picotest]`

```rust
use rstest::rstest;

#[rstest]
fn test_without_picotest_macro() {
    let cluster = picotest::run_cluster(".", 0);
    assert!(cluster.is_ok());
    assert!(cluster.is_ok_and(|cluster| cluster.path == "."))
}
```
