# Idlemail

Idlemail is a clone of fetchmail, focused around more modern usage scenarios, such as the support for an arbitrary amount of IDLE connections for low forwarding latency.

The architecture of Idlemail is fairly simple.
Split into sources, destinations, and the `MailHub`, which connects between them:

```
##########            ###########  mail due for reattempt ##############
# Source #  new mail  #         # <---------------------- #            #
# ------ # ---------> # MAILHUB #                         # RetryAgent #
#  src0  #            #         #  queue mail for retry   #            #
##########            ########### ----------------------> ##############
                     / \    ^
            deliver-/   \   |-sending failed
                   v     v  |
     ###############     ###############
     # Destination #     # Destination #
     # ----------- #     # ----------- #
     #    dst0     #     #    dst1     #
     ###############     ###############
```

* [Sources](#sources)
    * [Imap(Poll)](#ImapPoll)
    * [Imap(IDLE)](#ImapIDLE)
* [Destinations](#destinations)
    * [Smtp](#smtp)
    * [Exec](#exec)
* [RetryAgents](#RetryAgents)
    * [Memory](#memory)
    * [Filesystem](#filesystem)

---

# Sources
Sources are (as the name states), the sources for incoming mails.
Idlemail currently supports the following source implementations:

## ImapPoll
This source uses the IMAP protocoll, by regularly polling for new unread mails in the whole source account recursively.
- Downloaded mails are marked as read
- Downloaded mails can optionally be deleted from the account

#### Configuration parameters
- **interval**: Interval in seconds with which to poll. (Bear in mind that the IMAP server might terminate and block connections, when polling is done too often). The larger this interval is chosen, the longer the delay between incoming incoming mails and their retrieval can be.

##  ImapIDLE
This source uses the IMAP protocoll's IDLE extension, and thus only works within one mailbox (folder) in the account. When it starts, all unread mails in the configured mailbox are downloaded. Then, the source enters the IDLE state - waiting for the IMAP server to notify Idlemail about new mails. This, unforunately, only works within one mailbox, but allows the lowest possible delay between incoming mails and their retrieval.
- Downloaded mails are marked as read
- Downloaded mails can optionally be deleted from the account

#### Configuration parameters
- `path`: This is the path to the mailbox (folder) in the account, within which to wait/scan for incoming mails. Paths are `/` delimited. This limitation is due to the corresponding limitation of IMAP's IDLE extension.
- `renewinterval`: The interval with which the IDLE connection is refreshed. If this is too long, Idlemail could be classified as inactive, thus regularly kicked out of the connection. This interval is used to refresh the connection with the IMAP server. A typical value here (from the original RFC) is 29 minutes `=~1700`.

# Destinations
Destinations are (as the name states), the destinations, to which the mails retrieved through the sources should be delivered.
Idlemail currently supports the following destination implementations:

## Smtp
This destination uses the SMTP protocoll to deliver retrieved mails.
Bear in mind, that you will most probably have to use authenticated SMTP, to be able to deliver a mail, which was originally sent from *a* to *b*, into a destination account *c*.

#### Configuration parameters
- `ssl`: Whether to use ssl (`true`) or plain/startls (`false`)
- `recipient`: Mail address to deliver the mails to on the destination server

## Exec
This destination uses a binary on the local filesystem to deliver the mail. One instance of the binary is spawned for each mail. The mail is piped into the stdin stream of the spawned binary.
The child process inherits the environment variables of idlemail.
Additionally to that, idlemail sets some custom environment variables with information about the mail:

#### Special Environment Variables
- `IDLEMAIL_SOURCE`: Set to the configured name of the source, from which the mail came
- `IDLEMAIL_DESTINATION`: Name of the destination for which the binary is executed. This e.g. allows re-using the same executable for multiple destinations, even if some specific logic is required per destination.

#### Configuration parameters
- `executable`: Path to the executable to spawn for each mail
- \[`arguments`\]: Optional string array of arguments to pass to the exectuable
- \[`environment`\]: Optional Hashmap (json object) of environment variables that should be set additionally to, or overwrite variables inherited from idlemail's environment.

## Configuration
Configuration of Idlemail is done using a json configuration file.
For a complete example configuration file, have a look at `exampleconfig.json`.

The overall structure of the configuration file is:
```
{
    "destinations": {
        // Map of <destination name> to <configuration>
        "<destination name>": {
            // config
        }
    },
    "sources": {
        // Map of <source name> to <configuration>
        "<source name>": {
            // config
        }
    },
    "mappings": {
        // Mappings that encode, to which destination the mails of which source are forwarded
        "<source name>": [
            // list of
            "<destination name>"
        ]
    },
    "retryagent": { // optional
        // Configure the RetryAgent, which will try to re-schedule mails
        // that were not sent, e.g. due to a temporary Destination failure
    }
}
```

# RetryAgents
Idlemail also employs the concept of RetryAgents.
If a mail was downloaded from the source, it is gone. When the sending to some destination for such a mail fails, it is permanently lost.

Destinations thus report back to the `MailHub` if sending of a mail failed. The `MailHub` will then queue the mail into the optionally employed RetryAgent.
This RetryAgent remembers the mail, and to which destination it is supposed to go. After a configured amount of time, sending is re-attempted.

If a mail should have been distributed to multiple destinations, of which only one failed, only the delivery to this destination will be attempted.

Currently implemented RetryAgents are:

## Memory
RetryAgent that only stores messages in RAM.
If Idlemail is shut down while this RetryAgent has mails in queue, the mails will most definitely be lost.

#### Configuration parameters
- `delay`: Amount of seconds to wait until submitting the mail for a re-attempted sending.

## Filesystem
RetryAgent that is an extension of the Memory agent.
This agent stores mail in RAM, but also stores them in a designated (configured) folder in the filesystem.
If Idlemail is restarted, this RetryAgent will restore the previous queue from the filesystm folder.

#### Configuration parameters
- `delay`: Amount of seconds to wait until submitting the mail for a re-attempted sending.
- `path`: Path to a folder in the filesystem, where this RetryAgent will save mails to and restore them from when starting.