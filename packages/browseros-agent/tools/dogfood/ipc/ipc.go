package ipc

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"time"
)

const (
	CmdStatus  = "status"
	CmdStop    = "stop"
	CmdRestart = "restart"

	serverReadTimeout      = 5 * time.Second
	defaultResponseTimeout = 15 * time.Minute
)

var ErrDaemonNotRunning = errors.New("browseros-dogfood background daemon is not running")

type Request struct {
	Command string            `json:"command"`
	Args    map[string]string `json:"args,omitempty"`
}

type Response struct {
	OK    bool   `json:"ok"`
	Data  any    `json:"data,omitempty"`
	Error string `json:"error,omitempty"`
}

type Handler interface {
	Handle(Request) Response
}

type HandlerFunc func(Request) Response

func (f HandlerFunc) Handle(req Request) Response {
	return f(req)
}

type Server struct {
	socketPath string
	handler    Handler
	listener   net.Listener
}

func NewServer(socketPath string, handler Handler) *Server {
	return &Server{socketPath: socketPath, handler: handler}
}

func (s *Server) Start() error {
	if err := os.MkdirAll(filepath.Dir(s.socketPath), 0755); err != nil {
		return err
	}
	if _, err := os.Stat(s.socketPath); err == nil {
		conn, dialErr := net.DialTimeout("unix", s.socketPath, 300*time.Millisecond)
		if dialErr == nil {
			conn.Close()
			return fmt.Errorf("daemon socket is already active: %s", s.socketPath)
		}
		if err := os.Remove(s.socketPath); err != nil {
			return err
		}
	}
	listener, err := net.Listen("unix", s.socketPath)
	if err != nil {
		return err
	}
	s.listener = listener
	if err := os.Chmod(s.socketPath, 0600); err != nil {
		listener.Close()
		return err
	}
	go s.accept()
	return nil
}

func (s *Server) Stop() {
	if s.listener != nil {
		s.listener.Close()
	}
	os.Remove(s.socketPath)
}

func (s *Server) accept() {
	for {
		conn, err := s.listener.Accept()
		if err != nil {
			return
		}
		go s.handle(conn)
	}
}

func (s *Server) handle(conn net.Conn) {
	defer conn.Close()
	_ = conn.SetReadDeadline(time.Now().Add(serverReadTimeout))
	scanner := bufio.NewScanner(conn)
	if !scanner.Scan() {
		return
	}
	_ = conn.SetReadDeadline(time.Time{})
	var req Request
	if err := json.Unmarshal(scanner.Bytes(), &req); err != nil {
		writeResponse(conn, Response{Error: "invalid request"})
		return
	}
	writeResponse(conn, s.handler.Handle(req))
}

type Client struct {
	socketPath      string
	responseTimeout time.Duration
}

func NewClient(socketPath string) *Client {
	return NewClientWithTimeout(socketPath, defaultResponseTimeout)
}

func NewClientWithTimeout(socketPath string, responseTimeout time.Duration) *Client {
	return &Client{socketPath: socketPath, responseTimeout: responseTimeout}
}

func (c *Client) Send(req Request) (Response, error) {
	conn, err := net.DialTimeout("unix", c.socketPath, 700*time.Millisecond)
	if err != nil {
		return Response{}, ErrDaemonNotRunning
	}
	defer conn.Close()

	data, err := json.Marshal(req)
	if err != nil {
		return Response{}, err
	}
	data = append(data, '\n')
	if _, err := conn.Write(data); err != nil {
		return Response{}, err
	}
	if c.responseTimeout > 0 {
		_ = conn.SetReadDeadline(time.Now().Add(c.responseTimeout))
	}
	scanner := bufio.NewScanner(conn)
	if !scanner.Scan() {
		if err := scanner.Err(); err != nil {
			return Response{}, err
		}
		return Response{}, errors.New("daemon closed connection without a response")
	}
	var resp Response
	if err := json.Unmarshal(scanner.Bytes(), &resp); err != nil {
		return Response{}, err
	}
	return resp, nil
}

func writeResponse(conn net.Conn, resp Response) {
	data, err := json.Marshal(resp)
	if err != nil {
		data, _ = json.Marshal(Response{Error: "internal response error"})
	}
	data = append(data, '\n')
	_, _ = conn.Write(data)
}
