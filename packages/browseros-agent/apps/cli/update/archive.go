package update

import (
	"archive/tar"
	"archive/zip"
	"bytes"
	"compress/gzip"
	"context"
	"fmt"
	"io"
	"net/http"
)

const maxAssetSize = 64 << 20
const maxBinarySize = 256 << 20

func DownloadAsset(ctx context.Context, client *http.Client, asset Asset) ([]byte, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, asset.URL, nil)
	if err != nil {
		return nil, err
	}

	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("update download returned HTTP %d", resp.StatusCode)
	}

	return readAssetBytes(resp.Body)
}

func readAssetBytes(reader io.Reader) ([]byte, error) {
	limited := io.LimitReader(reader, maxAssetSize+1)
	data, err := io.ReadAll(limited)
	if err != nil {
		return nil, err
	}
	if len(data) > maxAssetSize {
		return nil, fmt.Errorf("update asset exceeds %d bytes", maxAssetSize)
	}
	return data, nil
}

func ExtractBinary(archive []byte, format string) ([]byte, error) {
	switch format {
	case "tar.gz":
		return extractTarGzBinary(archive)
	case "zip":
		return extractZipBinary(archive)
	default:
		return nil, fmt.Errorf("unsupported archive format %q", format)
	}
}

func extractTarGzBinary(archive []byte) ([]byte, error) {
	gzipReader, err := gzip.NewReader(bytes.NewReader(archive))
	if err != nil {
		return nil, err
	}
	defer gzipReader.Close()

	tarReader := tar.NewReader(gzipReader)
	return readTarBinary(tarReader)
}

func readTarBinary(reader *tar.Reader) ([]byte, error) {
	var binary []byte

	for {
		header, err := reader.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			return nil, err
		}
		if header.Typeflag != tar.TypeReg {
			continue
		}
		if binary != nil {
			return nil, fmt.Errorf("archive contains multiple files; expected exactly one binary")
		}

		binary, err = io.ReadAll(io.LimitReader(reader, maxBinarySize+1))
		if err != nil {
			return nil, err
		}
		if len(binary) > maxBinarySize {
			return nil, fmt.Errorf("extracted binary exceeds %d bytes", maxBinarySize)
		}
	}

	if binary == nil {
		return nil, fmt.Errorf("archive does not contain a file")
	}

	return binary, nil
}

func extractZipBinary(archive []byte) ([]byte, error) {
	reader, err := zip.NewReader(bytes.NewReader(archive), int64(len(archive)))
	if err != nil {
		return nil, err
	}

	var binary []byte
	for _, file := range reader.File {
		if file.FileInfo().IsDir() {
			continue
		}
		if binary != nil {
			return nil, fmt.Errorf("archive contains multiple files; expected exactly one binary")
		}

		rc, err := file.Open()
		if err != nil {
			return nil, err
		}
		binary, err = io.ReadAll(io.LimitReader(rc, maxBinarySize+1))
		rc.Close()
		if err != nil {
			return nil, err
		}
		if len(binary) > maxBinarySize {
			return nil, fmt.Errorf("extracted binary exceeds %d bytes", maxBinarySize)
		}
	}

	if binary == nil {
		return nil, fmt.Errorf("archive does not contain a file")
	}

	return binary, nil
}
