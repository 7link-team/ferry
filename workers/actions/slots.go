package main

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"time"
)

// SQL leases coordinate servers. OS locks fence the actual workers even when a
// server is suspended beyond the SQL TTL. Never unlink lock files: a replacement
// inode would let two workers believe they exclusively own the same slot.
func acquireExecutionSlot(ctx context.Context, request Request) (func(), error) {
	if request.ExecutionHost == "" {
		return func() {}, nil
	} // standalone callers
	if _, err := strconv.ParseUint(request.ExecutionHost, 10, 64); err != nil || request.ExecutionCapacity < 1 || request.ExecutionCapacity > 64 || request.ExecutionSlot < 0 || request.ExecutionSlot >= request.ExecutionCapacity {
		return nil, fmt.Errorf("invalid execution slot")
	}
	root := "/tmp"
	if runtime.GOOS == "windows" {
		root = os.Getenv("ProgramData")
		if root == "" {
			return nil, fmt.Errorf("system shared data directory unavailable")
		}
	}
	dir := filepath.Join(root, ".host-execution-"+request.ExecutionHost)
	if err := os.MkdirAll(dir, 0700); err != nil {
		return nil, err
	}
	// A failed takeover fails closed; an old worker is never killed solely
	// because a database heartbeat timed out.
	wait, cancel := context.WithTimeout(ctx, 60*time.Second)
	defer cancel()
	for {
		release, err := tryExecutionSlot(dir, request.ExecutionSlot, request.ExecutionCapacity)
		if err != nil {
			return nil, err
		}
		if release != nil {
			return release, nil
		}
		select {
		case <-wait.Done():
			return nil, fmt.Errorf("execution capacity remains occupied on host: %w", wait.Err())
		case <-time.After(250 * time.Millisecond):
		}
	}
}

func tryExecutionSlot(dir string, slot, capacity int) (func(), error) {
	gate, err := tryFileLock(filepath.Join(dir, "allocation.lock"))
	if err != nil || gate == nil {
		return nil, err
	}
	defer gate()
	active := 0
	var selected func()
	for i := 0; i < 64; i++ {
		release, err := tryFileLock(filepath.Join(dir, strconv.Itoa(i)+".lock"))
		if err != nil {
			if selected != nil {
				selected()
			}
			return nil, err
		}
		if release == nil {
			active++
		} else if i == slot {
			selected = release
		} else {
			release()
		}
	}
	if active >= capacity || selected == nil {
		if selected != nil {
			selected()
		}
		return nil, nil
	}
	return selected, nil
}
