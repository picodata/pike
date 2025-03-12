# Maintenance guide

## Making a new release

1. Update master branch

   ```shell
   git checkout master && git pull
   ```

2. Update project version in `Cargo.toml`

   ```shell
   vim Cargo.toml
   ```

3. Update `Cargo.lock`

   ```shell
   cargo update
   ```

4. Update `CHANGELOG.md`

   ```shell
   vim CHANGELOG.md
   ```

5. Update pike version in template dependencies

   ```shell
   vim plugin_template/_Cargo.toml
   ```

6. Commit changed files

   ```shell
   git commit -m "chore: bump version" Cargo.toml Cargo.lock CHANGELOG.md plugin_template/_Cargo.toml
   ```

7. Make a new git tag

   ```shell
   git tag -a <NEW_VERSION>
   ```

8. Push all to upstream

   ```shell
   git push origin master --follow-tags
   ```

9. [Create](https://github.com/picodata/pike/releases/new) a new release specifying pushed tag
10. Go to internal picodata [project](https://git.picodata.io/picodata/plugin-docker-build-images/-/pipelines) and run new pipeline on main branch with default parameters.
