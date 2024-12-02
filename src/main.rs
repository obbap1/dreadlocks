use dreadlock_lib::lock::start_lock_manager;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    start_lock_manager("[::1]:50052").await
}
