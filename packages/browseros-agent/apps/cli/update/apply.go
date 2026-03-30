package update

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"strings"

	"github.com/minio/selfupdate"
)

func CheckPermissions(targetPath string) error {
	options := selfupdate.Options{TargetPath: targetPath}
	return options.CheckPermissions()
}

func VerifyChecksum(data []byte, expectedHex string) error {
	expected, err := decodeChecksum(expectedHex)
	if err != nil {
		return err
	}
	actual := sha256.Sum256(data)
	if !bytes.Equal(actual[:], expected) {
		return fmt.Errorf(
			"checksum mismatch: expected %s, got %s",
			hex.EncodeToString(expected),
			hex.EncodeToString(actual[:]),
		)
	}
	return nil
}

func ApplyBinary(binary []byte, targetPath string) error {
	options := selfupdate.Options{TargetPath: targetPath}
	err := selfupdate.Apply(bytes.NewReader(binary), options)
	if rollbackErr := selfupdate.RollbackError(err); rollbackErr != nil {
		return fmt.Errorf("update failed and rollback failed: %w", rollbackErr)
	}
	return err
}

func decodeChecksum(checksumHex string) ([]byte, error) {
	value := strings.TrimSpace(checksumHex)
	if value == "" {
		return nil, fmt.Errorf("missing checksum")
	}
	return hex.DecodeString(value)
}
