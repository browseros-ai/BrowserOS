package common

import (
	"context"
	"fmt"
	"os"

	"github.com/aws/aws-sdk-go-v2/aws"
	"github.com/aws/aws-sdk-go-v2/credentials"
	"github.com/aws/aws-sdk-go-v2/service/s3"
)

type R2Client struct {
	client *s3.Client
	bucket string
}

func NewR2Client(env EnvConfig) (*R2Client, error) {
	if !env.HasR2Config() {
		return nil, fmt.Errorf("R2 configuration not set")
	}

	endpoint := env.R2EndpointURL()
	if endpoint == "" {
		return nil, fmt.Errorf("R2 endpoint could not be derived from R2_ACCOUNT_ID")
	}

	cfg := aws.Config{
		Region:      "auto",
		Credentials: aws.NewCredentialsCache(credentials.NewStaticCredentialsProvider(env.R2AccessKeyID, env.R2SecretAccessKey, "")),
		EndpointResolverWithOptions: aws.EndpointResolverWithOptionsFunc(func(service, region string, options ...interface{}) (aws.Endpoint, error) {
			if service == s3.ServiceID {
				return aws.Endpoint{URL: endpoint, HostnameImmutable: true}, nil
			}
			return aws.Endpoint{}, &aws.EndpointNotFoundError{}
		}),
	}

	client := s3.NewFromConfig(cfg, func(o *s3.Options) {
		o.UsePathStyle = true
	})

	return &R2Client{
		client: client,
		bucket: env.R2Bucket,
	}, nil
}

func (c *R2Client) UploadFile(ctx context.Context, localPath string, r2Key string) error {
	f, err := os.Open(localPath)
	if err != nil {
		return fmt.Errorf("opening %s: %w", localPath, err)
	}
	defer f.Close()

	_, err = c.client.PutObject(ctx, &s3.PutObjectInput{
		Bucket: &c.bucket,
		Key:    aws.String(r2Key),
		Body:   f,
	})
	if err != nil {
		return fmt.Errorf("put object %s: %w", r2Key, err)
	}

	return nil
}
