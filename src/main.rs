#[actix_web::main]
async fn main() -> std::io::Result<()> {
    rust::run_server().await
}
