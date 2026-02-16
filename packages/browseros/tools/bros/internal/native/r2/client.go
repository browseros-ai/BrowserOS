package r2

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"

	"github.com/aws/aws-sdk-go-v2/aws"
	"github.com/aws/aws-sdk-go-v2/credentials"
	"github.com/aws/aws-sdk-go-v2/service/s3"
	"github.com/aws/aws-sdk-go-v2/service/s3/types"
	"github.com/aws/smithy-go"
)

const (
	R2AccountIDEnv       = "R2_ACCOUNT_ID"
	R2AccessKeyIDEnv     = "R2_ACCESS_KEY_ID"
	R2SecretAccessKeyEnv = "R2_SECRET_ACCESS_KEY"
	R2BucketEnv          = "R2_BUCKET"
	R2CDNBaseURLEnv      = "R2_CDN_BASE_URL"
)

const (
	DefaultR2Bucket     = "browseros"
	DefaultR2CDNBaseURL = "http://cdn.browseros.com"
)

var ErrNotFound = errors.New("r2 object not found")

type Config struct {
	AccountID       string
	AccessKeyID     string
	SecretAccessKey string
	Bucket          string
	CDNBaseURL      string
	EndpointURL     string
}

type Client struct {
	s3         *s3.Client
	bucket     string
	cdnBaseURL string
}

func NewClientFromEnv() (*Client, error) {
	return NewClient(LoadConfigFromEnv())
}

func NewClient(cfg Config) (*Client, error) {
	if err := cfg.Validate(); err != nil {
		return nil, err
	}

	awsCfg := aws.Config{
		Region:      "auto",
		Credentials: aws.NewCredentialsCache(credentials.NewStaticCredentialsProvider(cfg.AccessKeyID, cfg.SecretAccessKey, "")),
	}

	s3Client := s3.NewFromConfig(awsCfg, func(opts *s3.Options) {
		opts.BaseEndpoint = aws.String(cfg.EndpointURL)
		opts.UsePathStyle = true
	})

	return &Client{
		s3:         s3Client,
		bucket:     cfg.Bucket,
		cdnBaseURL: strings.TrimRight(cfg.CDNBaseURL, "/"),
	}, nil
}

func LoadConfigFromEnv() Config {
	return LoadConfig(os.Getenv)
}

func LoadConfig(lookup func(string) string) Config {
	accountID := strings.TrimSpace(lookup(R2AccountIDEnv))
	accessKey := strings.TrimSpace(lookup(R2AccessKeyIDEnv))
	secretKey := strings.TrimSpace(lookup(R2SecretAccessKeyEnv))
	bucket := strings.TrimSpace(lookup(R2BucketEnv))
	cdnBase := strings.TrimSpace(lookup(R2CDNBaseURLEnv))

	if bucket == "" {
		bucket = DefaultR2Bucket
	}
	if cdnBase == "" {
		cdnBase = DefaultR2CDNBaseURL
	}

	cfg := Config{
		AccountID:       accountID,
		AccessKeyID:     accessKey,
		SecretAccessKey: secretKey,
		Bucket:          bucket,
		CDNBaseURL:      cdnBase,
	}
	if accountID != "" {
		cfg.EndpointURL = fmt.Sprintf("https://%s.r2.cloudflarestorage.com", accountID)
	}

	return cfg
}

func (c Config) HasConfig() bool {
	return c.AccountID != "" && c.AccessKeyID != "" && c.SecretAccessKey != ""
}

func (c Config) Validate() error {
	missing := make([]string, 0, 3)
	if c.AccountID == "" {
		missing = append(missing, R2AccountIDEnv)
	}
	if c.AccessKeyID == "" {
		missing = append(missing, R2AccessKeyIDEnv)
	}
	if c.SecretAccessKey == "" {
		missing = append(missing, R2SecretAccessKeyEnv)
	}
	if len(missing) > 0 {
		return fmt.Errorf("R2 configuration not set (missing: %s)", strings.Join(missing, ", "))
	}
	if c.EndpointURL == "" {
		return fmt.Errorf("R2 endpoint URL is empty")
	}
	if c.Bucket == "" {
		return fmt.Errorf("R2 bucket is empty")
	}
	return nil
}

func (c *Client) Bucket() string {
	return c.bucket
}

func (c *Client) CDNBaseURL() string {
	return c.cdnBaseURL
}

func (c *Client) ListReleaseVersions(ctx context.Context) ([]string, error) {
	versions := make(map[string]struct{})
	var token *string

	for {
		resp, err := c.s3.ListObjectsV2(ctx, &s3.ListObjectsV2Input{
			Bucket:            aws.String(c.bucket),
			Prefix:            aws.String("releases/"),
			Delimiter:         aws.String("/"),
			ContinuationToken: token,
		})
		if err != nil {
			return nil, err
		}

		for _, prefix := range resp.CommonPrefixes {
			version := strings.TrimSuffix(strings.TrimPrefix(aws.ToString(prefix.Prefix), "releases/"), "/")
			if version != "" {
				versions[version] = struct{}{}
			}
		}

		if !aws.ToBool(resp.IsTruncated) {
			break
		}
		token = resp.NextContinuationToken
	}

	out := make([]string, 0, len(versions))
	for version := range versions {
		out = append(out, version)
	}
	SortVersionsDesc(out)
	return out, nil
}

func SortVersionsDesc(versions []string) {
	sort.Slice(versions, func(i, j int) bool {
		return CompareVersions(versions[i], versions[j]) > 0
	})
}

// CompareVersions compares semantic-like versions.
// Returns 1 when a > b, -1 when a < b, and 0 when equal.
func CompareVersions(a, b string) int {
	aParts := trimTrailingZeros(parseVersionParts(a))
	bParts := trimTrailingZeros(parseVersionParts(b))

	limit := len(aParts)
	if len(bParts) > limit {
		limit = len(bParts)
	}

	for i := 0; i < limit; i++ {
		aVal := 0
		if i < len(aParts) {
			aVal = aParts[i]
		}
		bVal := 0
		if i < len(bParts) {
			bVal = bParts[i]
		}

		if aVal > bVal {
			return 1
		}
		if aVal < bVal {
			return -1
		}
	}

	return 0
}

func parseVersionParts(version string) []int {
	parts := strings.Split(version, ".")
	out := make([]int, 0, len(parts))
	for _, part := range parts {
		value, err := strconv.Atoi(part)
		if err != nil {
			value = 0
		}
		out = append(out, value)
	}
	return out
}

func trimTrailingZeros(values []int) []int {
	last := len(values) - 1
	for last >= 0 && values[last] == 0 {
		last--
	}
	if last < 0 {
		return []int{0}
	}
	return values[:last+1]
}

func (c *Client) FetchReleaseJSON(ctx context.Context, version, platform string) ([]byte, error) {
	key := fmt.Sprintf("releases/%s/%s/release.json", version, platform)
	return c.GetObject(ctx, key)
}

func (c *Client) GetObject(ctx context.Context, key string) ([]byte, error) {
	resp, err := c.s3.GetObject(ctx, &s3.GetObjectInput{
		Bucket: aws.String(c.bucket),
		Key:    aws.String(key),
	})
	if err != nil {
		if isNoSuchKey(err) {
			return nil, ErrNotFound
		}
		return nil, err
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}
	return body, nil
}

func (c *Client) CopyObject(ctx context.Context, sourceKey, destinationKey string) error {
	_, err := c.s3.CopyObject(ctx, &s3.CopyObjectInput{
		Bucket:     aws.String(c.bucket),
		Key:        aws.String(destinationKey),
		CopySource: aws.String(copySource(c.bucket, sourceKey)),
	})
	return err
}

func (c *Client) DownloadObject(ctx context.Context, key, destinationPath string) error {
	resp, err := c.s3.GetObject(ctx, &s3.GetObjectInput{
		Bucket: aws.String(c.bucket),
		Key:    aws.String(key),
	})
	if err != nil {
		if isNoSuchKey(err) {
			return ErrNotFound
		}
		return err
	}
	defer resp.Body.Close()

	if err := os.MkdirAll(filepath.Dir(destinationPath), 0o755); err != nil {
		return err
	}

	file, err := os.Create(destinationPath)
	if err != nil {
		return err
	}
	defer file.Close()

	if _, err := io.Copy(file, resp.Body); err != nil {
		return err
	}

	return nil
}

func copySource(bucket, key string) string {
	escapedKey := strings.ReplaceAll(url.PathEscape(key), "%2F", "/")
	return bucket + "/" + escapedKey
}

func isNoSuchKey(err error) bool {
	var notFound *types.NoSuchKey
	if errors.As(err, &notFound) {
		return true
	}

	var apiErr smithy.APIError
	if errors.As(err, &apiErr) {
		code := apiErr.ErrorCode()
		return code == "NoSuchKey" || code == "NotFound" || code == "404"
	}

	return false
}
