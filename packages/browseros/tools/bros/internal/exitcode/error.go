package exitcode

import "fmt"

// Error carries an explicit process exit code.
type Error struct {
	code int
	msg  string
}

func New(code int, msg string) *Error {
	if code <= 0 {
		code = 1
	}
	return &Error{code: code, msg: msg}
}

func (e *Error) Error() string {
	if e.msg != "" {
		return e.msg
	}
	return fmt.Sprintf("exit status %d", e.code)
}

func (e *Error) ExitCode() int {
	return e.code
}

func (e *Error) HasMessage() bool {
	return e.msg != ""
}
