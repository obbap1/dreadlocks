# dreadlocks
GRPC-based lock server with sqlite as the backing store. This implements the redis redlock algorithm for a single instance. 

# usage
Process A locks name "newLock" and Processes B and C get "LOCK_HELD" errors till process A explicitly unlocks this "newLock" resource or if a TTL is set.