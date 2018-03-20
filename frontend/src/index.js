import React from 'react';
import ReactDOM from 'react-dom';
import Moment from 'moment';
import DateTime from 'react-datetime';
import TweetEmbed from 'react-tweet-embed'
import './index.css';
var request = require('request');

class Twimeline extends React.Component {
  constructor(props) {
    super(props);
    this.state = {tweets: props.tweets};
    this.timers = [];
    this.scheduleTweetDisplay();
  }

  componentDidUpdate(prevProps, prevState) {
    if (prevProps === this.props) {
      return;
    }
    this.cancelTimers();
    this.setState({tweets: []});
    this.scheduleTweetDisplay();
  }

  componentWillUnmount() {
    this.cancelTimers();
  }

  scheduleTweetDisplay() {
    this.props.tweets.forEach((tweet, i) => {
      let delay = tweet.seconds_since_start / this.props.rate;
      console.log("Scheduling tweet " + tweet.id + " for in " + delay + " seconds");
      let timer = setTimeout(tweet => {
        this.setState((state, props) => {
          let tweets = state.tweets.slice();
          tweets.splice(0, 0, tweet.id);
          return {tweets: tweets};
        });
      }, 1000 * delay, tweet);
      this.timers.push(timer);
    })
  }

  cancelTimers() {
    this.timers.forEach(timer => clearInterval(timer));
    this.timers = [];
  }

  render() {
    if (this.state.tweets.length === 0) {
      return null;
    }
    return <div>{this.state.tweets.map(tweet => <TweetEmbed id={tweet} key={tweet} />)}</div>;
  }
}

class Form extends React.Component {
  constructor(props) {
    super(props);
    this.state = {
      who: "",
      from: null,
      until: null,
      rate: 1,
    };

    this.handleWhoChange = this.handleWhoChange.bind(this);
    this.handleRateChange = this.handleRateChange.bind(this);
    this.setDefaultUntil = this.setDefaultUntil.bind(this);
    this.handleSubmit = this.handleSubmit.bind(this);
  }

  handleWhoChange(event) {
    this.setState({who: event.target.value});
  }

  handleRateChange(event) {
    this.setState({rate: parseFloat(event.target.value)});
  }

  setDefaultUntil(from) {
    this.setState({from: from});
    if (typeof from === 'object' && !this.state.until) {
      this.setState({until: from.clone().add(1, 'hours')});
    }
  }

  handleSubmit(event) {
    // TODO: Validate values
    try {
      this.props.onSubmit(this.state.who, this.state.from, this.state.until, this.state.rate);
    } catch (e) {
      console.log(e);
    }
    event.preventDefault();
  }

  render() {
    return (
      <form method="GET" onSubmit={this.handleSubmit}>
        Who: <input name="who" type="text" value={this.state.who} onChange={this.handleWhoChange} /><br />
        From (UTC): <DateTime name="from" dateFormat="YYYY-MM-DD" timeFormat="HH:mm:ss" value={this.state.from} onChange={this.setDefaultUntil} /><br />
        Until (UTC): <DateTime name="until" dateFormat="YYYY-MM-DD" timeFormat="HH:mm:ss" value={this.state.until} /> <br />
        Rate: <input type="text" name="rate" value={this.state.rate} onChange={this.handleRateChange} /><br />
        <input type="submit" value="Play" />
      </form>
    );
  }
}

class App extends React.Component {
  constructor(props) {
    super(props);
    this.state = {
      error: null,
      tweets: [],
      rate: 1,
      requestId: 0,
    };
    this.updateTweets = this.updateTweets.bind(this);
  }

  updateTweets(who, from, until, rate) {
    request(window.location.protocol + "//" + window.location.host + "/feed/" + who + "/" + (from / 1000) + "/" + (until / 1000), (error, response, body) => {
      if (error || response.statusCode !== 200) {
        this.setError(error || response.statusCode);
        return;
      }
      try {
        let content = JSON.parse(body);
        this.setState(prevState => ({
          error: null,
          tweets: content,
          rate: rate,
          requestId: prevState.requestId + 1,
        }));
      } catch (e) {
        this.setError(e);
        return;
      }
    })
  }

  setError(error) {
    console.log(error);
    this.setState(prevState => ({
      error: "An error occurred fetching tweets",
      requestId: prevState.requestId + 1,
    }));
  }

  render() {
    let body = this.state.error ? <div style={{marginTop: '10px'}}>{this.state.error}</div> : <Twimeline tweets={this.state.tweets} rate={this.state.rate} requestId={this.state.requestId} />;
    return (
      <div>
        <Form onSubmit={this.updateTweets} />
        {body}
      </div>
    );
  }
}

ReactDOM.render(<App />, document.getElementById('root'));
