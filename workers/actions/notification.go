package main

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"maps"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/nektos/act/pkg/model"
	"gopkg.in/yaml.v3"
)

// Credentials never enter the worker: the server sends through the saved channel.
func rewriteBuiltinNotificationStep(step *model.Step, nonce, dir string) error {
	if step.Uses != "ferry/notify@v1" {
		return nil
	}
	executable, err := os.Executable()
	if err != nil {
		return err
	}
	env := maps.Clone(step.Environment())
	if env == nil {
		env = map[string]string{}
	}
	if len(step.With) > 106 {
		return fmt.Errorf("too many notification inputs")
	}
	keys := slices.Sorted(maps.Keys(step.With))
	for index, key := range keys {
		env[fmt.Sprintf("FERRY_NOTIFICATION_KEY_%d", index)] = key
		env[fmt.Sprintf("INPUT_FERRY_NOTIFICATION_%d", index)] = step.With[key]
	}
	env["FERRY_NOTIFICATION_INPUT_COUNT"] = strconv.Itoa(len(keys))
	env["FERRY_NOTIFICATION_EXECUTABLE"] = executable
	env["FERRY_NOTIFICATION_NONCE"] = nonce
	env["FERRY_NOTIFICATION_DIR"] = dir
	var node yaml.Node
	if err := node.Encode(env); err != nil {
		return err
	}
	step.Env = node
	if step.Name == "" {
		step.Name = step.Uses
	}
	step.Uses, step.With = "", nil
	step.Shell = "sh"
	step.Run = `"$FERRY_NOTIFICATION_EXECUTABLE" notify`
	if runtime.GOOS == "windows" {
		step.Shell = "cmd"
		step.Run = "@\"%FERRY_NOTIFICATION_EXECUTABLE%\" notify\r\n@exit /b %errorlevel%"
	}
	return nil
}

type notificationMessage struct {
	ClientID   string            `json:"client_id"`
	Subject    string            `json:"subject"`
	Body       string            `json:"body"`
	Format     string            `json:"format"`
	To         []string          `json:"to"`
	TemplateID string            `json:"template_id,omitempty"`
	Variables  map[string]string `json:"variables"`
}
type notificationRequest struct {
	ID        string              `json:"id"`
	Response  string              `json:"response"`
	ChannelID string              `json:"channel_id"`
	Message   notificationMessage `json:"message"`
}
type notificationResponse struct {
	DeliveryID string `json:"delivery_id"`
	State      string `json:"state"`
	Error      string `json:"error"`
}

func notificationInputs(input string) (notificationRequest, error) {
	var values map[string]string
	var request notificationRequest
	if len(input) > 64000 || json.Unmarshal([]byte(input), &values) != nil {
		return request, fmt.Errorf("invalid notification inputs")
	}
	request.ChannelID = strings.TrimSpace(values["channel-id"])
	if request.ChannelID == "" {
		return request, fmt.Errorf("channel-id is required")
	}
	request.Message = notificationMessage{Subject: values["subject"], Body: values["body"], Format: values["format"], TemplateID: values["template-id"], Variables: map[string]string{}, To: []string{}}
	if request.Message.Format == "" {
		request.Message.Format = "text"
	}
	for key, value := range values {
		if strings.HasPrefix(key, "variable-") {
			request.Message.Variables[strings.TrimPrefix(key, "variable-")] = value
			continue
		}
		switch key {
		case "channel-id", "template-id", "subject", "body", "format", "to":
		default:
			return request, fmt.Errorf("unsupported notification input: %s", key)
		}
	}
	for _, address := range strings.FieldsFunc(values["to"], func(r rune) bool { return r == '\n' || r == '\r' || r == ',' || r == ';' }) {
		if address = strings.TrimSpace(address); address != "" {
			request.Message.To = append(request.Message.To, address)
		}
	}
	if request.Message.Format != "text" && request.Message.Format != "html" {
		return request, fmt.Errorf("format must be text or html")
	}
	if request.Message.TemplateID == "" && strings.TrimSpace(request.Message.Body) == "" {
		return request, fmt.Errorf("body or template-id is required")
	}
	return request, nil
}

func runNotificationHelper(output io.Writer) error {
	count, err := strconv.Atoi(os.Getenv("FERRY_NOTIFICATION_INPUT_COUNT"))
	if err != nil || count < 1 || count > 106 {
		return fmt.Errorf("invalid notification inputs")
	}
	values := map[string]string{}
	for index := 0; index < count; index++ {
		values[os.Getenv(fmt.Sprintf("FERRY_NOTIFICATION_KEY_%d", index))] = os.Getenv(fmt.Sprintf("INPUT_FERRY_NOTIFICATION_%d", index))
	}
	inputs, err := json.Marshal(values)
	if err != nil {
		return err
	}
	request, err := notificationInputs(string(inputs))
	if err != nil {
		return err
	}
	nonce, dir := os.Getenv("FERRY_NOTIFICATION_NONCE"), os.Getenv("FERRY_NOTIFICATION_DIR")
	if nonce == "" || dir == "" {
		return fmt.Errorf("ferry/notify needs a job started by the service")
	}
	var id [8]byte
	if _, err := rand.Read(id[:]); err != nil {
		return err
	}
	request.ID = "notify-" + hex.EncodeToString(id[:])
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	request.Response = dir + "/" + request.ID + ".json"
	data, err := json.Marshal(request)
	if err != nil || len(data) > 65536 {
		return fmt.Errorf("notification request is too large")
	}
	fmt.Fprintln(output, "Requesting notification delivery")
	fmt.Fprintf(output, "::ferry-notify::%s::%s\n", nonce, data)
	deadline := time.Now().Add(60 * time.Second)
	for {
		var response notificationResponse
		data, err := os.ReadFile(filepath.FromSlash(request.Response))
		if err == nil && json.Unmarshal(data, &response) == nil {
			if response.Error != "" {
				return fmt.Errorf("%s", response.Error)
			}
			if response.State == "succeeded" && response.DeliveryID != "" {
				fmt.Fprintln(output, "Notification delivered")
				return nil
			}
		}
		if time.Now().After(deadline) {
			return fmt.Errorf("delivery result is unknown; check the notification record before sending again")
		}
		time.Sleep(previewPoll)
	}
}
