package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"testing"
	"time"
)

func TestExecutionSlotRequestUnlimitedAndConfiguredBeyond64(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Setenv("ProgramData", t.TempDir())
	}
	host := strconv.FormatInt(time.Now().UnixNano(), 10)
	root := "/tmp"
	if runtime.GOOS == "windows" {
		root = os.Getenv("ProgramData")
	}
	dir := filepath.Join(root, ".host-execution-"+host)
	t.Cleanup(func() { _ = os.RemoveAll(dir) }) // this test's unique host only
	for _, capacity := range []string{"null", "129", strconv.Itoa(int(^uint(0) >> 1))} {
		var request Request
		if err := json.Unmarshal([]byte(fmt.Sprintf(`{"execution_host":%q,"execution_slot":128,"execution_capacity":%s}`, host, capacity)), &request); err != nil {
			t.Fatal(err)
		}
		release, err := acquireExecutionSlot(context.Background(), request)
		if err != nil || release == nil {
			t.Fatalf("capacity %s slot128: %v", capacity, err)
		}
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Millisecond)
		duplicate, err := acquireExecutionSlot(ctx, request)
		cancel()
		if err == nil || duplicate != nil {
			release()
			t.Fatalf("occupied slot did not honor cancellation: %v", err)
		}
		release()
		replacement, err := acquireExecutionSlot(context.Background(), request)
		if err != nil || replacement == nil {
			t.Fatalf("released slot unavailable: %v", err)
		}
		replacement()
	}
	entries, err := os.ReadDir(dir)
	if err != nil || len(entries) != 2 {
		t.Fatalf("allocation created unrequested slot files: %d, %v", len(entries), err)
	}
	for _, request := range []Request{
		{ExecutionHost: host, ExecutionCapacity: -1},
		{ExecutionHost: host, ExecutionSlot: -1},
		{ExecutionHost: host, ExecutionSlot: 129, ExecutionCapacity: 129},
	} {
		if release, err := acquireExecutionSlot(context.Background(), request); err == nil || release != nil {
			t.Fatalf("invalid request accepted: %+v", request)
		}
	}
}

func TestExecutionSlotsDrainEveryOldHighSlot(t *testing.T) {
	dir := t.TempDir()
	var old []func()
	defer func() {
		for _, release := range old {
			if release != nil {
				release()
			}
		}
	}()
	for _, slot := range []int{64, 128, 1000000} {
		release, err := tryExecutionSlot(dir, slot, 1000001)
		if err != nil || release == nil {
			t.Fatalf("old slot %d: %v", slot, err)
		}
		old = append(old, release)
	}
	// Noncanonical names and directories are not execution slots, even if locked.
	for _, name := range []string{"01.lock", "-1.lock", "notes.lock"} {
		release, err := tryFileLock(filepath.Join(dir, name))
		if err != nil || release == nil {
			t.Fatal(name, err)
		}
		defer release()
	}
	if err := os.Mkdir(filepath.Join(dir, "2.lock"), 0700); err != nil {
		t.Fatal(err)
	}
	for remaining := 3; remaining > 0; remaining-- {
		release, err := tryExecutionSlot(dir, 0, 1)
		if err != nil || release != nil {
			t.Fatalf("reduction ignored %d active high slots: %v", remaining, err)
		}
		old[remaining-1]()
		old[remaining-1] = nil
	}
	release, err := tryExecutionSlot(dir, 0, 1)
	if err != nil || release == nil {
		t.Fatalf("drained capacity unavailable: %v", err)
	}
	release()
	if _, err := os.Stat(filepath.Join(dir, "1000000.lock")); err != nil {
		t.Fatal("old lock file removed", err)
	}
}

func TestExecutionSlotsFenceExpiredSQLAndCapacityReduction(t *testing.T) {
	dir := t.TempDir()
	old, err := tryExecutionSlot(dir, 1, 2)
	if err != nil || old == nil {
		t.Fatalf("old slot: %v", err)
	}
	defer old()
	// SQL may now say the old owner expired. OS ownership must still win.
	duplicate, err := tryExecutionSlot(dir, 1, 2)
	if err != nil || duplicate != nil {
		t.Fatalf("duplicate entered: %v", err)
	}
	replacement, err := tryExecutionSlot(dir, 0, 1)
	if err != nil || replacement != nil {
		t.Fatalf("reduced capacity overlapped higher old slot: %v", err)
	}
	parallel, err := tryExecutionSlot(dir, 0, 2)
	if err != nil || parallel == nil {
		t.Fatalf("independent capacity unavailable: %v", err)
	}
	parallel()
}

func TestExecutionSlotChild(t *testing.T) {
	dir := os.Getenv("EXECUTION_LOCK_TEST_DIR")
	if dir == "" {
		return
	}
	release, err := tryExecutionSlot(dir, 0, 1)
	if err != nil || release == nil {
		t.Fatal("child could not acquire", err)
	}
	defer release()
	fmt.Println("LOCKED")
	_, _ = bufio.NewReader(os.Stdin).ReadString('\n')
}

func TestExecutionSlotsCrossProcessAndCrashRecovery(t *testing.T) {
	dir := t.TempDir()
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()
	child := testCommandContext(ctx, os.Args[0], "-test.run=^TestExecutionSlotChild$")
	child.Env = append(os.Environ(), "EXECUTION_LOCK_TEST_DIR="+dir)
	input, err := child.StdinPipe()
	if err != nil {
		t.Fatal(err)
	}
	output, err := child.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err = child.Start(); err != nil {
		t.Fatal(err)
	}
	defer func() { _ = child.Process.Kill(); _ = child.Wait() }()
	ready, err := bufio.NewReader(output).ReadString('\n')
	if err != nil || ready != "LOCKED\n" {
		t.Fatalf("child readiness %q: %v", ready, err)
	}
	release, err := tryExecutionSlot(dir, 0, 1)
	if err != nil || release != nil {
		t.Fatalf("parallel process entered: %v", err)
	}
	// Lock files persist; the kernel, not deletion or SQL TTL, releases ownership.
	_ = child.Process.Kill()
	_ = child.Wait()
	_ = input.Close()
	release, err = tryExecutionSlot(dir, 0, 1)
	if err != nil || release == nil {
		t.Fatalf("crashed process did not release: %v", err)
	}
	release()
	if _, err := os.Stat(filepath.Join(dir, "0.lock")); err != nil {
		t.Fatal(err)
	}
}
