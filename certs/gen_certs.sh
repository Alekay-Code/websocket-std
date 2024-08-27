# Create folder structure
mkdir -p rootCA
mkdir -p example
mkdir -p newcerts
touch db.txt 
touch rand.txt
mkdir -p serial

echo "
[ req ]
default_bits = 4096
default_md = sha256
default_keyfile = private.key
prompt = no
encrypt_key = no

distinguished_name = req_distinguished_name
x509_extension = v3_ca

[ req_distinguished_name ]
C = US
ST = California
L = San Francisco
O = My company
OU = My Division
CN = My Root CA

[ v3_ca ]
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid:always,issuer
basicConstraints = CA:TRUE
keyUsage = critical, digitalSignature, cRLSign, keyCertSign
" > rootCA/openssl_root.cfg

# Create private key for rootCA
openssl genrsa -out rootCA/private.key 4096

# Create cert for rootCA
openssl req -config rootCA/openssl_root.cfg -new -x509 -nodes -keyout rootCA/private.key -out rootCA/cert.pem -days 3650

# Convert rootCA cert pem format into der format
openssl x509 -inform pem -outform der -in rootCA/cert.pem -out rootCA/cert.der

echo "
[ req ]
default_bits = 4096
default_md = sha256
default_keyfile = example.com.key
prompt = no
encrypt_key = no

distinguished_name = req_distinguished_name
x509_extension = v3_ca

[ req_distinguished_name ]
C = US
ST = California
L = San Francisco
O = My company
OU = My Division
CN = My Root CA

[ v3_ca ]
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid:always,issuer
basicConstraints = CA:FALSE
keyUsage = critical, digitalSignature, keyEncipherment 
extendedKeyUsage = serverAuth
subjectAltName = @alt_names

[ alt_name ]
DNS.1 = example.com
DNS.2 = www.example.com
" > example/example.cfg

# Create private key for example.com
openssl genrsa -out example/example.com.key 4096 

# Create sign request
openssl req -config example/example.cfg -new -key example/example.com.key -out example/example.com.csr


echo "
[ ca ]
default_ca = CA_default

[ CA_default ]
certs = ./example
new_certs_dir = ./newcerts
database = ./db.txt
rand_serial = ./serial
RANDFILE = ./rand.txt
default_md = sha256
private_key = rootCA/private.key
certificate = rootCA/cert.pem
policy = policy_match
email_in_dn = no

[ policy_match ]
countryName = match
stateOrProvinceName = match
organizationName = match
organizationalUnitName = optional
commonName = supplied
emailAddress = optional

[ v3_req ]
subjectKeyIdentifier = hash
# authorityKeyIdentifier = keyid:always,issuer
basicConstraints = CA:FALSE
keyUsage = critical, digitalSignature, keyEncipherment 
extendedKeyUsage = serverAuth
subjectAltName = @alt_names

[ alt_names ]
DNS.1 = example.com
DNS.2 = www.example.com
" > rootCA/openssl_ca.cfg

# Sign cert
# openssl x509 -req -in example/example.com.csr -CA rootCA/cert.pem -CAkey rootCA/private.key -CAcreateserial -out example/example.com.crt -days 365
openssl ca -config rootCA/openssl_ca.cfg -in example/example.com.csr -out example/example.com.crt -extensions v3_req -days 365

# der format
openssl x509 -inform pem -outform der -in example/example.com.crt -out example/example.com.der