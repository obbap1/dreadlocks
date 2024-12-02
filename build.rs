fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("proto/lock.proto")?;
    let connection = sqlite::Connection::open("locks.db")?;
    connection
        .execute(
            "
            CREATE TABLE IF NOT EXISTS locks (
                name TEXT PRIMARY KEY,
                random_id TEXT NOT NULL,
                expiry INTEGER,
                is_locked INTEGER
            );
            ",
        )?;
    Ok(())
}