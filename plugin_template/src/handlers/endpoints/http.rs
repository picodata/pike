use std::collections::HashMap;

use shors::transport::{
    Context,
    http::{
        Request, Response,
        route::{Builder, Route},
    },
};

#[must_use]
pub fn routes() -> Vec<Route<anyhow::Error>> {
    let hello_route = Builder::new().with_method("GET").with_path("/hello").build(
        |_: &mut Context, _: Request| -> anyhow::Result<_> {
            let message: &str = "Hello there! This is Pike. Use cargo pike --help for more tips.";
            Ok(Response {
                status: 200,
                headers: HashMap::from([(
                    "content-type".to_string(),
                    "text/plain; charset=utf8".to_string(),
                )]),
                body: message.as_bytes().to_vec(),
            })
        },
    );

    vec![hello_route]
}
