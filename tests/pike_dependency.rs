mod helpers;

use helpers::{
    assert_pike_build_dependency_resolves_to_local_pike_project, cleanup_dir, exec_pike_in,
    rewrite_project_and_workspace_member_pike_build_dependencies_to_local_pike_project, TESTS_DIR,
};
use rstest::rstest;
use std::fs;
use std::path::{Path, PathBuf};

struct TestPluginProjectsDirectory {
    projects_directory: PathBuf,
}

impl TestPluginProjectsDirectory {
    fn create() -> Self {
        let projects_directory = Path::new(TESTS_DIR).join(format!(
            "pike-dependency-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir(&projects_directory).unwrap();
        Self { projects_directory }
    }

    fn projects_directory(&self) -> &Path {
        &self.projects_directory
    }
}

impl Drop for TestPluginProjectsDirectory {
    fn drop(&mut self) {
        cleanup_dir(&self.projects_directory);
    }
}

fn create_project_directory_with_cargo_file(project_directory: &Path, cargo_contents: &str) {
    fs::create_dir_all(project_directory).unwrap();
    fs::write(project_directory.join("Cargo.toml"), cargo_contents).unwrap();
}

#[test]
fn repeated_pike_dependency_preparation_preserves_cargo_bytes() {
    let plugin_projects = TestPluginProjectsDirectory::create();
    let plugin_directory = plugin_projects.projects_directory();
    create_project_directory_with_cargo_file(
        plugin_directory,
        "[build-dependencies]\npicodata-pike = \"0.0.0-unpublished-test\"\n",
    );
    let cargo_path = plugin_directory.join("Cargo.toml");
    rewrite_project_and_workspace_member_pike_build_dependencies_to_local_pike_project(
        plugin_directory.as_os_str(),
    );
    let prepared_cargo_bytes = fs::read(&cargo_path).unwrap();

    rewrite_project_and_workspace_member_pike_build_dependencies_to_local_pike_project(
        plugin_directory.as_os_str(),
    );

    assert_eq!(fs::read(&cargo_path).unwrap(), prepared_cargo_bytes);
}

#[test]
fn prepares_workspace_members_without_changing_workspace_root_cargo() {
    let plugin_projects = TestPluginProjectsDirectory::create();
    let workspace_directory = plugin_projects.projects_directory();
    create_project_directory_with_cargo_file(
        workspace_directory,
        "[workspace]\nmembers = [\"first\", \"second\"]\n",
    );
    let first_member_directory = workspace_directory.join("first");
    create_project_directory_with_cargo_file(
        &first_member_directory,
        "[build-dependencies]\npicodata-pike = \"0.0.0-unpublished-test\"\n",
    );
    let second_member_directory = workspace_directory.join("second");
    create_project_directory_with_cargo_file(
        &second_member_directory,
        "[build-dependencies]\npicodata-pike = \"0.0.0-unpublished-test\"\n",
    );
    let workspace_cargo_path = workspace_directory.join("Cargo.toml");
    let original_workspace_cargo_bytes = fs::read(&workspace_cargo_path).unwrap();

    rewrite_project_and_workspace_member_pike_build_dependencies_to_local_pike_project(
        workspace_directory.as_os_str(),
    );

    assert_eq!(
        fs::read(&workspace_cargo_path).unwrap(),
        original_workspace_cargo_bytes
    );
    for member_directory in [&first_member_directory, &second_member_directory] {
        assert_pike_build_dependency_resolves_to_local_pike_project(member_directory);
    }
}

#[test]
fn preserves_cargo_bytes_outside_pike_build_dependency() {
    let plugin_projects = TestPluginProjectsDirectory::create();
    let cargo_contents = "# original formatting\n\
        [dependencies]\n\
        picodata-pike = \"0.0.0-keep-this-dependency\"\n\
        \n\
        [build-dependencies]\n\
        picodata-pike = \"0.0.0-unpublished-test\" # keep this comment\n";
    let plugin_directory = plugin_projects.projects_directory();
    create_project_directory_with_cargo_file(plugin_directory, cargo_contents);

    rewrite_project_and_workspace_member_pike_build_dependencies_to_local_pike_project(
        plugin_directory.as_os_str(),
    );

    assert_eq!(
        fs::read_to_string(plugin_directory.join("Cargo.toml")).unwrap(),
        cargo_contents.replace("\"0.0.0-unpublished-test\"", "{ path = \"../../..\" }")
    );
}

#[test]
#[should_panic(expected = "missing-plugin/Cargo.toml")]
fn reports_missing_expected_cargo_file() {
    let plugin_projects = TestPluginProjectsDirectory::create();
    let plugin_directory = plugin_projects.projects_directory().join("missing-plugin");
    rewrite_project_and_workspace_member_pike_build_dependencies_to_local_pike_project(
        plugin_directory.as_os_str(),
    );
}

#[test]
#[should_panic(expected = "missing-plugin/Cargo.toml")]
fn reports_missing_workspace_member_cargo_file() {
    let plugin_projects = TestPluginProjectsDirectory::create();
    let workspace_directory = plugin_projects.projects_directory();
    create_project_directory_with_cargo_file(
        workspace_directory,
        "[workspace]\nmembers = [\"missing-plugin\"]\n",
    );
    rewrite_project_and_workspace_member_pike_build_dependencies_to_local_pike_project(
        workspace_directory.as_os_str(),
    );
}

#[test]
#[should_panic(expected = "failed to parse")]
fn reports_invalid_cargo_toml() {
    let plugin_projects = TestPluginProjectsDirectory::create();
    let plugin_directory = plugin_projects.projects_directory();
    create_project_directory_with_cargo_file(plugin_directory, "[build-dependencies\n");
    rewrite_project_and_workspace_member_pike_build_dependencies_to_local_pike_project(
        plugin_directory.as_os_str(),
    );
}

#[test]
fn prepares_project_immediately_after_plugin_new() {
    let plugin_projects = TestPluginProjectsDirectory::create();

    exec_pike_in(
        ["plugin", "new", "created-plugin"],
        plugin_projects.projects_directory(),
    );

    assert_pike_build_dependency_resolves_to_local_pike_project(
        &plugin_projects.projects_directory().join("created-plugin"),
    );
}

#[rstest]
#[case::working_directory(false)]
#[case::explicit_plugin_path(true)]
fn prepares_workspace_members_immediately_after_plugin_add(#[case] explicit_plugin_path: bool) {
    let plugin_projects = TestPluginProjectsDirectory::create();
    exec_pike_in(
        ["plugin", "new", "workspace", "--workspace"],
        plugin_projects.projects_directory(),
    );
    let workspace_directory = plugin_projects.projects_directory().join("workspace");

    if explicit_plugin_path {
        exec_pike_in(
            [
                "plugin",
                "add",
                "added-plugin",
                "--plugin-path",
                "workspace",
            ],
            plugin_projects.projects_directory(),
        );
    } else {
        exec_pike_in(["plugin", "add", "added-plugin"], &workspace_directory);
    }

    assert_pike_build_dependency_resolves_to_local_pike_project(
        &workspace_directory.join("workspace"),
    );
    assert_pike_build_dependency_resolves_to_local_pike_project(
        &workspace_directory.join("added-plugin"),
    );
}

#[test]
fn prepares_project_immediately_after_plugin_init() {
    let plugin_projects = TestPluginProjectsDirectory::create();
    let plugin_directory = plugin_projects
        .projects_directory()
        .join("initialized-plugin");
    fs::create_dir(&plugin_directory).unwrap();

    exec_pike_in(["plugin", "init"], &plugin_directory);

    assert_pike_build_dependency_resolves_to_local_pike_project(&plugin_directory);
}

#[test]
fn does_not_prepare_projects_when_creation_command_only_displays_help() {
    let plugin_projects = TestPluginProjectsDirectory::create();

    for plugin_subcommand in ["new", "init"] {
        exec_pike_in(
            ["plugin", plugin_subcommand, "--help"],
            plugin_projects.projects_directory(),
        );
    }

    assert!(fs::read_dir(plugin_projects.projects_directory())
        .unwrap()
        .next()
        .is_none());
}
