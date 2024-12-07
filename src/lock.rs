use sqlite::{Connection, State};
use tonic::{transport::Server, Request, Response, Status};

use lock_manager::lock_manager_server::{LockManager, LockManagerServer};
use lock_manager::{GenericReply, LockRequest, UnlockRequest};
use once_cell::sync::Lazy;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

pub mod lock_manager {
    tonic::include_proto!("locker");
}

pub enum ErrCodes {
    SystemError = 1,
    ClientError = 2,
    LockError = 3,
}

pub const INVALID_SQL_QUERY: &str = "INVALID_SQL_QUERY";
pub const LOCK_HELD: &str = "LOCK_HELD";
pub const INVALID_RANDOM_ID: &str = "INVALID_RANDOM_ID";
pub const LOCK_DOES_NOT_EXIST: &str = "LOCK_DOES_NOT_EXIST";
pub const LOCK_ALREADY_RELEASED: &str = "LOCK_ALREADY_RELEASED";
pub const THIS_CLIENT_DOESNT_HOLD_LOCK: &str = "THIS_CLIENT_DOESNT_HOLD_LOCK";

static CONNECTION: Lazy<Arc<Mutex<Connection>>> = Lazy::new(|| {
    let connection = Connection::open("locks.db").unwrap();
    Arc::new(Mutex::new(connection))
});

fn calculate_expiry(ttl: u64, now: u64) -> i64 {
    if ttl > 0 {
        (now + ttl) as i64
    } else {
        0
    }
}

fn insert_lock_record(
    connection: &MutexGuard<'_, Connection>,
    req: &LockRequest,
    now: u64,
) -> Result<State, sqlite::Error> {
    let query = "
            INSERT INTO locks VALUES (?, ?, ?, 1)
        ";
    let mut statement = connection.prepare(query).unwrap();
    statement.bind((1, req.name.as_str())).unwrap();
    statement.bind((2, req.random_id.as_str())).unwrap();
    statement.bind((3, calculate_expiry(req.ttl, now))).unwrap();

    statement.next()
}

fn update_lock_record(
    connection: &MutexGuard<'_, Connection>,
    req: &LockRequest,
    now: u64,
) -> Result<State, sqlite::Error> {
    let query = "UPDATE locks SET is_locked = 1, random_id = ?, expiry = ? where name = ?";
    let mut statement = connection.prepare(query).unwrap();

    statement.bind((1, req.random_id.as_str())).unwrap();
    statement.bind((2, calculate_expiry(req.ttl, now))).unwrap();
    statement.bind((3, req.name.as_str())).unwrap();

    statement.next()
}

fn construct_response(
    ok: bool,
    err_code: u64,
    err_message: &str,
) -> Result<tonic::Response<GenericReply>, Status> {
    Ok(Response::new(GenericReply {
        ok: Some(ok),
        err_code: Some(err_code),
        err_message: Some(err_message.to_string()),
    }))
}

#[derive(Debug, Default)]
pub struct MyLockManager {}

#[tonic::async_trait]
impl LockManager for MyLockManager {
    async fn lock(&self, request: Request<LockRequest>) -> Result<Response<GenericReply>, Status> {
        let req = request.into_inner();
        if req.random_id.is_empty() {
            return construct_response(false, ErrCodes::ClientError as u64, INVALID_RANDOM_ID);
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let query = "SELECT * FROM locks where name = ?";
        let connection = CONNECTION.lock().unwrap();
        let mut statement = connection.prepare(query).unwrap();

        statement.bind((1, req.name.as_str())).unwrap();

        let is_row = match statement.next() {
            Ok(State::Done) => false,
            Ok(State::Row) => true,
            Err(e) => {
                println!("error {:?}", e);
                return construct_response(false, ErrCodes::SystemError as u64, INVALID_SQL_QUERY);
            }
        };

        if !is_row {
            // insert new row
            match insert_lock_record(&connection, &req, now) {
                Ok(_) => {
                    return construct_response(true, 0, "");
                }
                Err(e) => {
                    println!("error insert_lock_record {:?}", e);
                    return construct_response(
                        false,
                        ErrCodes::SystemError as u64,
                        INVALID_SQL_QUERY,
                    );
                }
            }
        }

        let expiry = statement.read::<i64, _>("expiry").unwrap();
        let is_locked = statement.read::<i64, _>("is_locked").unwrap();

        if is_locked == 0 {
            // its not locked, update as locked
            match update_lock_record(&connection, &req, now) {
                Ok(_) => {
                    return construct_response(true, 0, "");
                }
                Err(e) => {
                    println!("error is_locked {:?}", e);
                    return construct_response(
                        false,
                        ErrCodes::SystemError as u64,
                        INVALID_SQL_QUERY,
                    );
                }
            }
        }

        if expiry == 0 || now < expiry as u64 {
            // 1. there's no expiry set so lock has to be released explicitly
            // 2. the expiry time hasn't been exceeded.
            // 3. its already locked
            return construct_response(false, ErrCodes::LockError as u64, LOCK_HELD);
        }

        // finally, update lock
        match update_lock_record(&connection, &req, now) {
            Ok(_) => {
                return construct_response(true, 0, "");
            }
            Err(e) => {
                println!("error update_lock_record {:?}", e);
                return construct_response(false, ErrCodes::SystemError as u64, INVALID_SQL_QUERY);
            }
        }
    }

    async fn unlock(
        &self,
        request: Request<UnlockRequest>,
    ) -> Result<Response<GenericReply>, Status> {
        let req = request.into_inner();
        if req.random_id.is_empty() {
            return construct_response(false, ErrCodes::ClientError as u64, INVALID_RANDOM_ID);
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let query = "SELECT * FROM locks where name = ?";
        let connection = CONNECTION.lock().unwrap();
        let mut statement = connection.prepare(query).unwrap();

        statement.bind((1, req.name.as_str())).unwrap();

        let is_row = match statement.next() {
            Ok(State::Done) => false,
            Ok(State::Row) => true,
            Err(_) => {
                return construct_response(false, ErrCodes::SystemError as u64, INVALID_SQL_QUERY);
            }
        };

        if !is_row {
            return construct_response(false, ErrCodes::ClientError as u64, LOCK_DOES_NOT_EXIST);
        }

        let expiry = statement.read::<i64, _>("expiry").unwrap();
        let is_locked = statement.read::<i64, _>("is_locked").unwrap();
        let random_id = statement.read::<String, _>("random_id").unwrap();

        if is_locked == 0 || expiry != 0 && now > expiry as u64 {
            return construct_response(false, ErrCodes::ClientError as u64, LOCK_ALREADY_RELEASED);
        }

        if random_id != req.random_id {
            return construct_response(
                false,
                ErrCodes::ClientError as u64,
                THIS_CLIENT_DOESNT_HOLD_LOCK,
            );
        }

        let query = "UPDATE locks SET is_locked = 0, random_id = ? where name = ?";
        let mut statement = connection.prepare(query).unwrap();

        statement.bind((1, "")).unwrap();
        statement.bind((2, req.name.as_str())).unwrap();

        match statement.next() {
            Ok(_) => {
                return construct_response(true, 0, "");
            }
            Err(_) => {
                return construct_response(false, ErrCodes::SystemError as u64, INVALID_SQL_QUERY);
            }
        }
    }
}

pub async fn start_lock_manager(addr: &str) -> Result<(), Box<dyn std::error::Error>> {
    let addr = addr.parse()?;
    let lock_manager: MyLockManager = MyLockManager::default();

    Server::builder()
        .add_service(LockManagerServer::new(lock_manager))
        .serve(addr)
        .await?;

    Ok(())
}

mod tests {
    use std::time::Duration;

    use super::*;
    // use crate::start_lock_manager;
    use lock_manager::lock_manager_client::LockManagerClient;

    #[tokio::test]
    async fn test_lock_manager() {
        let addr = "[::1]:50053";

        tokio::spawn(async {
            start_lock_manager(addr).await.unwrap();
        });

        tokio::time::sleep(Duration::from_secs(1)).await;

        let mut client = LockManagerClient::connect("http://".to_string() + addr)
            .await
            .unwrap();

        let lock_request = tonic::Request::new(lock_manager::LockRequest {
            name: "test_lock".to_string(),
            random_id: "seed".to_string(),
            ttl: 2,
        });

        let response = client.lock(lock_request).await.unwrap();
        // lock should be held
        assert!(response.into_inner().ok());

        // unlock with different random id
        let unlock_request = tonic::Request::new(lock_manager::UnlockRequest {
            name: "test_lock".to_string(),
            random_id: "seed-wrong".to_string(),
        });

        let response = client.unlock(unlock_request).await.unwrap();
        // unlock should fail
        let res = response.into_inner();
        assert!(!res.ok());
        assert_eq!(res.err_code(), ErrCodes::ClientError as u64);
        assert_eq!(res.err_message(), THIS_CLIENT_DOESNT_HOLD_LOCK);

        // sleep for 4 seconds (TTL is 2 )
        tokio::time::sleep(Duration::from_secs(4)).await;

        // call unlock again
        let unlock_request = tonic::Request::new(lock_manager::UnlockRequest {
            name: "test_lock".to_string(),
            random_id: "seed-wrong".to_string(),
        });

        let response = client.unlock(unlock_request).await.unwrap();
        // unlock should fail
        let res = response.into_inner();
        assert!(!res.ok());
        assert_eq!(res.err_code(), ErrCodes::ClientError as u64);
        // lock should be released!
        assert_eq!(res.err_message(), LOCK_ALREADY_RELEASED);
    }
}
