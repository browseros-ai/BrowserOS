package update

import (
	"archive/tar"
	"archive/zip"
	"bytes"
	"compress/gzip"
	"crypto/sha256"
	"encoding/hex"
	"os"
	"path/filepath"
	"testing"
)

func TestExtractBinaryTarGz(t *testing.T) {
	archive := createTarGz(t, map[string]string{"browseros-cli": "new-binary"})

	binary, err := ExtractBinary(archive, "tar.gz")
	if err != nil {
		t.Fatalf("ExtractBinary() error = %v", err)
	}
	if string(binary) != "new-binary" {
		t.Fatalf("ExtractBinary() = %q, want %q", string(binary), "new-binary")
	}
}

func TestExtractBinaryZip(t *testing.T) {
	archive := createZip(t, map[string]string{"browseros-cli.exe": "new-binary"})

	binary, err := ExtractBinary(archive, "zip")
	if err != nil {
		t.Fatalf("ExtractBinary() error = %v", err)
	}
	if string(binary) != "new-binary" {
		t.Fatalf("ExtractBinary() = %q, want %q", string(binary), "new-binary")
	}
}

func TestExtractBinaryTarGzRejectsMultipleFiles(t *testing.T) {
	archive := createTarGz(t, map[string]string{
		"browseros-cli":     "new-binary",
		"browseros-cli.sig": "signature",
	})

	_, err := ExtractBinary(archive, "tar.gz")
	if err == nil {
		t.Fatal("ExtractBinary() error = nil, want multiple files error")
	}
	if err.Error() != "archive contains multiple files; expected exactly one binary" {
		t.Fatalf("ExtractBinary() error = %q", err)
	}
}

func TestVerifyChecksumValid(t *testing.T) {
	data := []byte("some-data")
	sum := sha256.Sum256(data)
	if err := VerifyChecksum(data, hex.EncodeToString(sum[:])); err != nil {
		t.Fatalf("VerifyChecksum() error = %v", err)
	}
}

func TestVerifyChecksumMismatch(t *testing.T) {
	data := []byte("some-data")
	badChecksum := "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	if err := VerifyChecksum(data, badChecksum); err == nil {
		t.Fatal("VerifyChecksum() error = nil, want mismatch error")
	}
}

func TestApplyBinary(t *testing.T) {
	targetPath := filepath.Join(t.TempDir(), "browseros-cli")
	if err := os.WriteFile(targetPath, []byte("old-binary"), 0755); err != nil {
		t.Fatalf("WriteFile() error = %v", err)
	}

	newBinary := []byte("new-binary")
	if err := ApplyBinary(newBinary, targetPath); err != nil {
		t.Fatalf("ApplyBinary() error = %v", err)
	}

	data, err := os.ReadFile(targetPath)
	if err != nil {
		t.Fatalf("ReadFile() error = %v", err)
	}
	if string(data) != "new-binary" {
		t.Fatalf("updated binary = %q, want %q", string(data), "new-binary")
	}
}

func TestVerifyThenApplyIntegration(t *testing.T) {
	archive := createTarGz(t, map[string]string{"browseros-cli": "updated-binary"})
	archiveSum := sha256.Sum256(archive)

	if err := VerifyChecksum(archive, hex.EncodeToString(archiveSum[:])); err != nil {
		t.Fatalf("VerifyChecksum(archive) error = %v", err)
	}

	binary, err := ExtractBinary(archive, "tar.gz")
	if err != nil {
		t.Fatalf("ExtractBinary() error = %v", err)
	}

	targetPath := filepath.Join(t.TempDir(), "browseros-cli")
	if err := os.WriteFile(targetPath, []byte("old"), 0755); err != nil {
		t.Fatalf("WriteFile() error = %v", err)
	}
	if err := ApplyBinary(binary, targetPath); err != nil {
		t.Fatalf("ApplyBinary() error = %v", err)
	}

	data, err := os.ReadFile(targetPath)
	if err != nil {
		t.Fatalf("ReadFile() error = %v", err)
	}
	if string(data) != "updated-binary" {
		t.Fatalf("binary = %q, want %q", string(data), "updated-binary")
	}
}

func createTarGz(t *testing.T, files map[string]string) []byte {
	t.Helper()

	var buffer bytes.Buffer
	gzipWriter := gzip.NewWriter(&buffer)
	tarWriter := tar.NewWriter(gzipWriter)
	for name, body := range files {
		data := []byte(body)
		if err := tarWriter.WriteHeader(&tar.Header{
			Name: name,
			Mode: 0755,
			Size: int64(len(data)),
		}); err != nil {
			t.Fatalf("WriteHeader() error = %v", err)
		}
		if _, err := tarWriter.Write(data); err != nil {
			t.Fatalf("Write() error = %v", err)
		}
	}
	if err := tarWriter.Close(); err != nil {
		t.Fatalf("Close() error = %v", err)
	}
	if err := gzipWriter.Close(); err != nil {
		t.Fatalf("Close() error = %v", err)
	}

	return buffer.Bytes()
}

func createZip(t *testing.T, files map[string]string) []byte {
	t.Helper()

	var buffer bytes.Buffer
	zipWriter := zip.NewWriter(&buffer)
	for name, body := range files {
		fileWriter, err := zipWriter.Create(name)
		if err != nil {
			t.Fatalf("Create() error = %v", err)
		}
		if _, err := fileWriter.Write([]byte(body)); err != nil {
			t.Fatalf("Write() error = %v", err)
		}
	}
	if err := zipWriter.Close(); err != nil {
		t.Fatalf("Close() error = %v", err)
	}

	return buffer.Bytes()
}
