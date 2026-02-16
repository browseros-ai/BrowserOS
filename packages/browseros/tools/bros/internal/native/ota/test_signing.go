package ota

import (
	"fmt"
	"os"
	"path/filepath"

	"bros/internal/native/common"
)

func runTestSigning(args []string, packagesDir string) error {
	if len(args) != 1 {
		return fmt.Errorf("usage: bros ota test-signing <file>")
	}

	filePath := filepath.Clean(args[0])
	info, err := os.Stat(filePath)
	if err != nil {
		if os.IsNotExist(err) {
			return fmt.Errorf("file not found: %s", filePath)
		}
		return fmt.Errorf("checking file %s: %w", filePath, err)
	}
	if info.IsDir() {
		return fmt.Errorf("path is a directory: %s", filePath)
	}

	env := common.LoadEnv(packagesDir)
	signature, length, err := signWithSparkle(filePath, env)
	if err != nil {
		return err
	}

	preview := signature
	if len(preview) > 50 {
		preview = preview[:50] + "..."
	}

	fmt.Println("Signed successfully")
	fmt.Printf("Signature: %s\n", preview)
	fmt.Printf("Length: %d\n", length)
	return nil
}
