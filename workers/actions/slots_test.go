package main

import (
	"bufio"
	"context"
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"
)

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
