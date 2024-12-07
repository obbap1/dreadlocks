const grpc = require('@grpc/grpc-js');
const protoLoader = require('@grpc/proto-loader');
const cluster = require('cluster');
const { promisify } = require('util');

const PROTO_PATH = 'lock.proto';

// Load proto file
const packageDefinition = protoLoader.loadSync(PROTO_PATH, {
    keepCase: true,
    longs: String,
    enums: String,
    defaults: true,
    oneofs: true,
});

const proto = grpc.loadPackageDefinition(packageDefinition).locker;

// Server address (replace with your gRPC server's address)
const SERVER_ADDRESS = 'localhost:50052';

let counter = 0;

let testTTL = true;

if (cluster.isMaster) {
    const numWorkers = 4; // Number of worker processes
    console.log(`Master process is running. Forking ${numWorkers} workers...`);

    // Fork workers
    for (let i = 0; i < numWorkers; i++) {
        cluster.fork();
    }

    cluster.on('exit', (worker, code, signal) => {
        console.log(`Worker ${worker.process.pid} exited with code ${code} (${signal || 'no signal'})`);
    });
} else {
    const workerId = cluster.worker.id;
    console.log(`Worker ${workerId} started (PID: ${process.pid})`);

    const client = new proto.LockManager(SERVER_ADDRESS, grpc.credentials.createInsecure());

    const LockAsync = promisify(client.Lock).bind(client);

    const UnlockAsync = promisify(client.Lock).bind(client);

    // Simulate gRPC call and increment request
    async function makeGrpcCall() {
        const request = { name: `testLock`, random_id: workerId, ttl: 10 }; // use TTLs so you have a clean slate after restarts for example

        const lockRequest = await LockAsync(request);

        if (lockRequest.ok) {
            console.log(`Worker ${workerId} won the lock!. ->>>>> ${counter}`)
            // kill owner of the lock to see if TTL works
            if (testTTL) {
                process.exit(0);
            }
            const unlockRequest = await UnlockAsync(request)
            if (unlockRequest.ok) {
                console.log(`Worker ${workerId} won the lock!. ->>>>> ${counter}`)
                // kill owner of the lock to see if TTL works
                process.exit(0);
            } else if (unlockRequest.err_code > 0) {
                console.log(`Worker ${workerId} failed to drop lock with error code and message ${unlockRequest.err_code} -> ${unlockRequest.err_message}`)
            }
        }else if (lockRequest.err_code > 0){
            console.log(`Worker ${workerId} failed to acquire lock with error code and message ${lockRequest.err_code} -> ${lockRequest.err_message}`)
        }
    }
    // Call the gRPC server periodically
    setInterval(makeGrpcCall, (200 - workerId) * 10);
}