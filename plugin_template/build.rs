use pike::helpers::build;

fn main() {
    let params = build::ParamsBuilder::default().build().unwrap();
    build::main(&params);
}
