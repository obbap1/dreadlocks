use std::time::Duration;

use criterion::criterion_group;
use criterion::criterion_main;
use criterion::BenchmarkId;
use criterion::Criterion;
use criterion::Throughput;

use dreadlock_lib::lock::{
    lock_manager::{
        lock_manager_client::LockManagerClient, GenericReply, LockRequest, UnlockRequest,
    },
    start_lock_manager,
};
use std::fmt::Write;
use futures::future;
use rand::distributions::Alphanumeric;
use rand::Rng;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

enum Req {
    Lock(LockRequest, mpsc::Sender<GenericReply>),
    Unlock(UnlockRequest, mpsc::Sender<GenericReply>),
}

fn locks(c: &mut Criterion) {
    // Create a Tokio runtime
    let rt = Runtime::new().unwrap();

    let (tx, mut rx) = mpsc::channel(32);

    rt.block_on(async move {
        tokio::spawn(async {
            start_lock_manager("[::1]:50053").await.unwrap();
        });

        tokio::time::sleep(Duration::from_secs(1)).await;

        tokio::spawn(async move {
            let mut client = LockManagerClient::connect("http://".to_string() + "[::1]:50053")
                .await
                .unwrap();

            while let Some(cmd) = rx.recv().await {
                match cmd {
                    Req::Lock(lock_request, sender) => {
                        let l = tonic::Request::new(lock_request);
                        let response = client.lock(l).await.unwrap();

                        sender.send(response.into_inner()).await.unwrap();
                    }
                    Req::Unlock(unlock_request, sender) => {
                        let u = tonic::Request::new(unlock_request);
                        let response = client.unlock(u).await.unwrap();

                        sender.send(response.into_inner()).await.unwrap();
                    }
                }
            }
        });
    });

    let mut group = c.benchmark_group("lock_manager");
    for num_tasks in [5, 10, 20, 50, 100, 150, 200].iter() {
        group.throughput(Throughput::Elements(*num_tasks as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_tasks),
            num_tasks,
            |b, &num_tasks| {
                b.iter(|| {
                    rt.block_on(async {
                        let handles: Vec<_> = (0..=num_tasks)
                            .map(|t| {
                                let new_tx = tx.clone();
                                tokio::spawn(async move {
                                    
                                    let rando: String = rand::thread_rng()
                                        .sample_iter(&Alphanumeric)
                                        .take(16)
                                        .map(char::from)
                                        .collect();
                                    let mut lock_name = "try_lock_me_".to_string();
                                    write!(lock_name, "{}", num_tasks).unwrap();
                                    let (resp_tx, mut resp_rx) = mpsc::channel(1);
                                    let cmd = Req::Lock(
                                        LockRequest {
                                            name: lock_name.clone(),
                                            random_id: rando.clone(),
                                            ttl: 2,
                                        },
                                        resp_tx.clone(),
                                    );

                                    new_tx.send(cmd).await.unwrap();

                                    let res = resp_rx.recv().await;
                                    println!("response 1 {} is {:?}", t, res);

                                    tokio::time::sleep(Duration::from_secs(t+1)).await;

                                    let cmd = Req::Unlock(
                                        UnlockRequest {
                                            name: lock_name,
                                            random_id: rando,
                                        },
                                        resp_tx,
                                    );

                                    new_tx.send(cmd).await.unwrap();

                                    let res = resp_rx.recv().await;
                                    println!("response 2 {} is {:?}", t, res);
                                })
                            })
                            .collect();
                        future::join_all(handles).await;
                        // client_manager.await.unwrap();
                        // lock_manager.await.unwrap();
                    })
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, locks);
criterion_main!(benches);
