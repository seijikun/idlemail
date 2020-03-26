# Idlemail

Idlemail is a clone of fetchmail, focused around more modern usage scenarios, such as the support for an arbitrary amount of IDLE connections for low forwarding latency.

The architecture of Idlemail is fairly simple.
Split into sources, destinations, and the `MailHub`, which connects between them.

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
- **path**: This is the path to the mailbox (folder) in the account, within which to wait/scan for incoming mails. Paths are `/` delimited. This limitation is due to the corresponding limitation of IMAP's IDLE extension.
- **renewinterval**: The interval with which the IDLE connection is refreshed. If this is too long, Idlemail could be classified as inactive, thus regularly kicked out of the connection. This interval is used to refresh the connection with the IMAP server. A typical value here (from the original RFC) is 29 minutes `=~1700`.

# Destinations
Destinations are (as the name states), the destinations, to which the mails retrieved through the sources should be delivered.
Idlemail currently supports the following destination implementations:

## Smtp
This source uses the SMTP protocoll, to deliver retrieved mails.
Bear in mind, that you will most probably have to use authenticated SMTP, to be able to deliver a mail, which was originally sent from *a* to *b*, into a destination account *c*.

#### Configuration parameters
- **ssl**: Whether to use ssl (`true`) or plain/startls (`false`)
- **recipient**: Mail address to deliver the mails to on the destination server

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
    }
}
```

## TODO:
The current architecture is too simple.
If sending of a mail fails, the mail is permanently lost (with `keep = false`), because it was already deleted from the source.
I see two ways to fix this:

### Permanent-Storage
Store the mail in permanent storage and try again... (when?)

### Two-Way Feedback architecture
Only marking mails in the source as read & deleted after they were successfully sent, using feedback from the destination back to the source.

```
<source> --download--> MailHub --distribute--> <destination>
destination attempts to send mail
<destination> --notify about success--> MailHub --notify about success--> <source>
<source> --delete mail--> <server>
```
This would allow only marking mails as read (& deleted), that were successfully sent; however, if there are two destinations and sending fails on only one of them, the mail will be duplicated on subsequent attempts.