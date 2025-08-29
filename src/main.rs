#[actix_web::main]
async fn main() -> std::io::Result<()> {
    coin_comic_tales_rs::run_server().await
}
