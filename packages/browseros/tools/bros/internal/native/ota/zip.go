package ota

import (
	"archive/zip"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
)

func createServerZip(binaryPath string, outputZip string, isWindows bool) error {
	stagingDir := filepath.Join(filepath.Dir(outputZip), "staging_"+strings.TrimSuffix(filepath.Base(outputZip), filepath.Ext(outputZip)))
	defer os.RemoveAll(stagingDir)

	binDir := filepath.Join(stagingDir, "resources", "bin")
	if err := os.MkdirAll(binDir, 0o755); err != nil {
		return fmt.Errorf("creating staging directory: %w", err)
	}

	targetName := "browseros_server"
	if isWindows {
		targetName = "browseros_server.exe"
	}

	targetBinary := filepath.Join(binDir, targetName)
	if err := copyFile(binaryPath, targetBinary); err != nil {
		return fmt.Errorf("copying binary into zip staging area: %w", err)
	}

	zipFile, err := os.Create(outputZip)
	if err != nil {
		return fmt.Errorf("creating zip file %s: %w", outputZip, err)
	}
	defer zipFile.Close()

	zipWriter := zip.NewWriter(zipFile)
	defer zipWriter.Close()

	if err := filepath.Walk(stagingDir, func(path string, info os.FileInfo, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		if info.IsDir() {
			return nil
		}

		relPath, err := filepath.Rel(stagingDir, path)
		if err != nil {
			return err
		}
		relPath = filepath.ToSlash(relPath)

		header, err := zip.FileInfoHeader(info)
		if err != nil {
			return err
		}
		header.Name = relPath
		header.Method = zip.Deflate
		header.SetMode(info.Mode())

		writer, err := zipWriter.CreateHeader(header)
		if err != nil {
			return err
		}

		file, err := os.Open(path)
		if err != nil {
			return err
		}

		if _, err := io.Copy(writer, file); err != nil {
			_ = file.Close()
			return err
		}
		return file.Close()
	}); err != nil {
		return fmt.Errorf("writing zip contents: %w", err)
	}

	return nil
}
