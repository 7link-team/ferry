package main

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"time"
)

// SQL leases coordinate servers. OS locks fence the actual workers even when a
// server is suspended beyond the SQL TTL. Never unlink lock files: a replacement
// inode would let two workers believe they exclusively own the same slot.
func acquireExecutionSlot(ctx context.Context, request Request) (func(), error) {
	if request.ExecutionHost == "" {
		return func() {}, nil
	} // standalone callers
	// JSON null decodes to zero: the server uses it for unlimited capacity.
	if _, err := strconv.ParseUint(request.ExecutionHost, 10, 64); err != nil || request.ExecutionCapacity < 0 || request.ExecutionSlot < 0 || (request.ExecutionCapacity > 0 && request.ExecutionSlot >= request.ExecutionCapacity) {
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
	selectedName := strconv.Itoa(slot) + ".lock"
	selected, err := tryFileLock(filepath.Join(dir, selectedName))
	if err != nil || selected == nil {
		return nil, err
	}
	if capacity == 0 {
		return selected, nil
	}
	// Scan existing slots, including old owners above a reduced capacity. Never
	// iterate up to capacity: configured capacities can be arbitrarily large.
	entries, err := os.ReadDir(dir)
	if err != nil {
		selected()
		return nil, err
	}
	active := 0
	for _, entry := range entries {
		name := entry.Name()
		if !entry.Type().IsRegular() || name == selectedName || !strings.HasSuffix(name, ".lock") {
			continue
		}
		number := strings.TrimSuffix(name, ".lock")
		index, err := strconv.ParseUint(number, 10, 64)
		if err != nil || strconv.FormatUint(index, 10) != number {
			continue
		}
		release, err := tryFileLock(filepath.Join(dir, name))
		if err != nil {
			selected()
			return nil, err
		}
		if release == nil {
			active++
			if active >= capacity {
				selected()
				return nil, nil
			}
		} else {
			release()
		}
	}
	return selected, nil
}
